use std::process::Command;
use std::sync::LazyLock;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::wire::{self, ProcessInfo};
use crate::log::{log, log_debug, log_warn};

#[derive(Debug, thiserror::Error)]
pub enum HerdrError {
    #[error("`herdr {args:?}` failed: {stderr}")]
    CommandFailed { args: Vec<String>, stderr: String },
    #[error("failed to spawn `herdr`: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse herdr response: {0}")]
    Parse(#[from] serde_json::Error),
}

/// Outcome of `pane wait-output`. `TimedOut` is treated identically to
/// `Matched` by callers — it doubles as the liveness heartbeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    Matched,
    TimedOut,
}

fn herdr_bin() -> &'static str {
    static BIN: OnceLock<String> = OnceLock::new();
    BIN.get_or_init(|| {
        std::env::var("HERDR_BIN_PATH").unwrap_or_else(|_| {
            log("HERDR_BIN_PATH not set, falling back to `herdr` on PATH");
            "herdr".to_owned()
        })
    })
}

fn run_herdr(args: &[&str]) -> Result<String, HerdrError> {
    let output = Command::new(herdr_bin()).args(args).output()?;
    if !output.status.success() {
        return Err(HerdrError::CommandFailed {
            args: args.iter().map(|s| (*s).to_owned()).collect(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

pub fn list_panes() -> Result<Vec<String>, HerdrError> {
    let raw = run_herdr(&["pane", "list"])?;
    log_debug(&format!(
        "raw 'pane list' response: {}",
        truncate(&raw, 2000)
    ));
    let parsed: Value = serde_json::from_str(&raw)?;
    let ids = wire::extract_pane_ids(&parsed);
    if ids.is_empty() && !raw.trim().is_empty() {
        log_warn(&format!(
            "could not find a pane array in 'pane list' response, got: {}",
            truncate(&raw, 500)
        ));
    }
    Ok(ids)
}

/// Every foreground process in the pane, unfiltered. Fetched once per
/// newly-seen pane and tried against every registered detector, instead
/// of each detector re-fetching.
pub fn get_pane_process_info(pane_id: &str) -> Result<Vec<ProcessInfo>, HerdrError> {
    let raw = run_herdr(&["pane", "process-info", "--pane", pane_id])?;
    log_debug(&format!(
        "raw 'pane process-info' response for {pane_id}: {}",
        truncate(&raw, 1000)
    ));
    let parsed: Value = serde_json::from_str(&raw)?;
    Ok(wire::extract_foreground_processes(&parsed))
}

pub fn read_pane_output(pane_id: &str, lines: u32) -> Result<String, HerdrError> {
    run_herdr(&[
        "pane",
        "read",
        pane_id,
        "--source",
        "recent-unwrapped",
        "--lines",
        &lines.to_string(),
    ])
}

/// Blocks until `regex` matches recent pane output or `timeout_ms` elapses.
/// UNVERIFIED: behavior when the pane closes mid-wait. Distinguishing
/// timeout from a hard "pane gone" error relies on
/// `wire::stderr_indicates_timeout`, itself unverified — confirm against a
/// live closed pane before relying on this in production.
pub fn wait_output(
    pane_id: &str,
    regex: &str,
    lines: u32,
    timeout_ms: u64,
) -> Result<WaitOutcome, HerdrError> {
    let timeout_str = timeout_ms.to_string();
    let lines_str = lines.to_string();
    let args = [
        "pane",
        "wait-output",
        pane_id,
        "--regex",
        regex,
        "--source",
        "recent-unwrapped",
        "--lines",
        &lines_str,
        "--timeout",
        &timeout_str,
    ];

    let output = Command::new(herdr_bin()).args(args).output()?;
    if output.status.success() {
        return Ok(WaitOutcome::Matched);
    }

    let stderr_raw = String::from_utf8_lossy(&output.stderr).into_owned();
    if let Ok(stderr_json) = serde_json::from_str::<Value>(&stderr_raw)
        && wire::stderr_indicates_timeout(&stderr_json)
    {
        return Ok(WaitOutcome::TimedOut);
    }

    Err(HerdrError::CommandFailed {
        args: args.iter().map(|s| (*s).to_owned()).collect(),
        stderr: stderr_raw,
    })
}

/// `--seq` lets Herdr ignore stale reports from the same `--source`.
/// `source` and `agent` are caller-supplied.
pub fn report_agent_state(
    pane_id: &str,
    source: &str,
    agent: &str,
    state: &str,
    message: Option<&str>,
    seq: u64,
) -> Result<(), HerdrError> {
    let seq_str = seq.to_string();
    let mut args = vec![
        "pane",
        "report-agent",
        pane_id,
        "--source",
        source,
        "--agent",
        agent,
        "--state",
        state,
        "--seq",
        &seq_str,
    ];
    if let Some(m) = message {
        args.push("--message");
        args.push(m);
    }
    run_herdr(&args)?;
    Ok(())
}

pub fn release_agent(pane_id: &str, source: &str, agent: &str, seq: u64) -> Result<(), HerdrError> {
    let seq_str = seq.to_string();
    run_herdr(&[
        "pane",
        "release-agent",
        pane_id,
        "--source",
        source,
        "--agent",
        agent,
        "--seq",
        &seq_str,
    ])?;
    Ok(())
}

pub struct MetadataUpdate<'a> {
    pub display_agent: Option<&'a str>,
    pub tokens: Vec<(&'a str, String)>,
    pub state_labels: Vec<(String, String)>,
}

pub fn report_metadata(
    pane_id: &str,
    source: &str,
    update: &MetadataUpdate,
    seq: u64,
) -> Result<(), HerdrError> {
    let seq_str = seq.to_string();
    let mut args: Vec<String> = vec![
        "pane".into(),
        "report-metadata".into(),
        pane_id.into(),
        "--source".into(),
        source.into(),
        "--seq".into(),
        seq_str,
    ];

    if let Some(agent) = update.display_agent {
        args.push("--display-agent".into());
        args.push(agent.into());
    }
    for (status, text) in &update.state_labels {
        args.push("--state-label".into());
        args.push(format!("{status}={text}"));
    }
    for (name, value) in &update.tokens {
        args.push("--token".into());
        args.push(format!("{name}={value}"));
    }

    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_herdr(&arg_refs)?;
    Ok(())
}

/// Clears every metadata value a detector sets on a pane (display name +
/// tokens), so a dead pane's sidebar row doesn't freeze on the last
/// reported status. `release_agent` alone does NOT do this — agent-state
/// authority and display metadata are two independent layers.
///
/// VERIFY exact `--clear-token` syntax against `herdr pane report-metadata
/// --help` on your installed version — not confirmed against a running
/// instance here.
pub fn clear_metadata(
    pane_id: &str,
    source: &str,
    token_names: &[String],
    seq: u64,
) -> Result<(), HerdrError> {
    let seq_str = seq.to_string();
    let mut args: Vec<String> = vec![
        "pane".into(),
        "report-metadata".into(),
        pane_id.into(),
        "--source".into(),
        source.into(),
        "--seq".into(),
        seq_str,
        "--clear-display-agent".into(),
        "--clear-state-labels".into(),
    ];
    for name in token_names {
        args.push("--clear-token".into());
        args.push(name.clone());
    }
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_herdr(&arg_refs)?;
    Ok(())
}

/// `kill(pid, 0)`: sends no signal, only checks the process exists and is
/// signalable by this user.
pub fn is_pid_alive(pid: u32) -> bool {
    // SAFETY: signal 0 is a documented no-op probe; pid is a plain integer,
    // no pointers or lifetimes involved.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

/// Monotonic for the life of this process, seeded from epoch millis so a
/// daemon restart still outruns whatever seq herdr last recorded. Shared
/// across ALL detectors, never reset per-pane or per-worker — herdr
/// silently drops any report whose seq isn't higher than the last one
/// seen for the same `--source`.
static SEQ_COUNTER: LazyLock<AtomicU64> = LazyLock::new(|| {
    let epoch_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    AtomicU64::new(epoch_ms)
});

pub fn next_seq() -> u64 {
    SEQ_COUNTER.fetch_add(1, Ordering::Relaxed)
}
