use std::sync::Arc;
use std::sync::mpsc::Sender;

use crate::herdr::client::{self, MetadataUpdate, WaitOutcome};
use crate::log::{log, log_error};
use crate::tools::{ToolDetector, ToolStatus};

const READ_LINES: u32 = 60;
const WAIT_TIMEOUT_MS: u64 = 5_000;

struct TrackedState {
    pid: u32,
    status: ToolStatus,
    has_errors: bool,
    tokens: Vec<(String, String)>,
}

/// One per confirmed pane. Runs until the pane's process dies or a hard
/// error indicates the pane itself is gone, then reports back via
/// `done_tx` so the pane_id becomes eligible for re-detection.
///
/// No local seq counter — every report uses `client::next_seq()`, the
/// process-wide counter shared across every detector/pane. A per-worker
/// counter restarting at 0 made every report after the first pane
/// instance look stale to herdr and get silently ignored — don't
/// reintroduce one.
pub fn run(
    pane_id: String,
    initial_pid: u32,
    detector: Arc<dyn ToolDetector>,
    done_tx: Sender<String>,
) {
    let mut state = TrackedState {
        pid: initial_pid,
        status: ToolStatus::Starting,
        has_errors: false,
        tokens: Vec::new(),
    };

    loop {
        match client::wait_output(
            &pane_id,
            detector.signal_regex().as_str(),
            READ_LINES,
            WAIT_TIMEOUT_MS,
        ) {
            Ok(WaitOutcome::Matched | WaitOutcome::TimedOut) => {
                if !client::is_pid_alive(state.pid) {
                    log(&format!(
                        "pane {pane_id}: tracked pid {} no longer alive, clearing metadata and releasing agent",
                        state.pid
                    ));
                    release(&pane_id, detector.as_ref());
                    break;
                }
                if let Err(err) = tick(&pane_id, detector.as_ref(), &mut state) {
                    log_error(&format!("pane {pane_id}: tick failed"), &err);
                }
            }
            Err(err) => {
                // The failed call IS the "pane closed" signal —
                // UNVERIFIED against a real closed pane, see client::wait_output.
                log_error(
                    &format!("pane {pane_id}: wait_output hard error, treating pane as gone"),
                    &err,
                );
                let _ = client::clear_metadata(
                    &pane_id,
                    detector.source(),
                    detector.clear_token_names(),
                    client::next_seq(),
                );
                let _ = client::release_agent(
                    &pane_id,
                    detector.source(),
                    detector.agent_name(),
                    client::next_seq(),
                );
                break;
            }
        }
    }

    let _ = done_tx.send(pane_id);
}

fn release(pane_id: &str, detector: &dyn ToolDetector) {
    if let Err(err) = client::clear_metadata(
        pane_id,
        detector.source(),
        detector.clear_token_names(),
        client::next_seq(),
    ) {
        log_error(&format!("pane {pane_id}: clear_metadata failed"), &err);
    }
    if let Err(err) = client::release_agent(
        pane_id,
        detector.source(),
        detector.agent_name(),
        client::next_seq(),
    ) {
        log_error(&format!("pane {pane_id}: release_agent failed"), &err);
    }
}

fn tick(
    pane_id: &str,
    detector: &dyn ToolDetector,
    state: &mut TrackedState,
) -> Result<(), crate::herdr::HerdrError> {
    let output = client::read_pane_output(pane_id, READ_LINES)?;
    let result = detector.match_output(&output, state.has_errors);

    let changed = result.status != state.status
        || result.has_errors != state.has_errors
        || result.extra_tokens != state.tokens;

    if !changed {
        return Ok(());
    }

    log(&format!(
        "pane {pane_id}: state change -> status={:?} has_errors={} tokens={:?}",
        result.status, result.has_errors, result.extra_tokens
    ));

    let agent_state = if result.has_errors {
        "blocked"
    } else if result.status == ToolStatus::Starting || result.is_building {
        "working"
    } else {
        "idle"
    };

    // Separate seq per call, NOT shared — herdr scopes "seq <= last
    // accepted is ignored" per --source, and report_agent_state /
    // report_metadata share a source here, so one seq value across both
    // risks the second call being dropped as a stale duplicate.
    if let Err(err) = client::report_agent_state(
        pane_id,
        detector.source(),
        detector.agent_name(),
        agent_state,
        None,
        client::next_seq(),
    ) {
        log_error(
            &format!("pane {pane_id}: failed to report agent state"),
            &err,
        );
    }

    let token_refs: Vec<(&str, String)> = result
        .extra_tokens
        .iter()
        .map(|(k, v)| (k.as_str(), v.clone()))
        .collect();

    let update = MetadataUpdate {
        display_agent: Some(detector.display_agent()),
        tokens: token_refs,
        state_labels: detector.state_labels().to_vec(),
    };
    if let Err(err) =
        client::report_metadata(pane_id, detector.source(), &update, client::next_seq())
    {
        log_error(&format!("pane {pane_id}: failed to report metadata"), &err);
    }

    state.status = result.status;
    state.has_errors = result.has_errors;
    state.tokens = result.extra_tokens;

    Ok(())
}
