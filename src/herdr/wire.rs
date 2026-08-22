//! Raw JSON response shapes only. No tool-matching here — that belongs to
//! each `tools::ToolDetector::confirm`, since "is this process the tool"
//! is a per‑tool question and this layer has no business answering it.

use serde_json::Value;

/// `pane list` can apparently return the array directly, or nested under
/// `result`, `panes`, or `result.panes`. Same tolerance as the original TS
/// version.
pub fn extract_pane_ids(parsed: &Value) -> Vec<String> {
    let candidates = [
        Some(parsed),
        parsed.get("result"),
        parsed.get("panes"),
        parsed.pointer("/result/panes"),
    ];

    for candidate in candidates.into_iter().flatten() {
        if let Some(array) = candidate.as_array() {
            return array
                .iter()
                .filter_map(|p| {
                    p.get("pane_id")
                        .or_else(|| p.get("id"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .collect();
        }
    }
    Vec::new()
}

/// One foreground process in a pane, unfiltered — argv as reported, no
/// assumption about which entry (if any) is a runtime binary or a script
/// path. Each detector decides what its own argv shape looks like.
#[derive(Debug, Default, Clone)]
pub struct ProcessInfo {
    pub pid: Option<u32>,
    pub argv: Vec<String>,
    pub cwd: Option<String>,
}

/// Every foreground process Herdr reports for a pane.
pub fn extract_foreground_processes(parsed: &Value) -> Vec<ProcessInfo> {
    let Some(foreground) = parsed
        .pointer("/result/process_info/foreground_processes")
        .and_then(Value::as_array)
    else {
        return Vec::new();
    };

    let fallback_cwd = parsed
        .pointer("/result/process_info/foreground_cwd")
        .and_then(Value::as_str);

    foreground
        .iter()
        .map(|proc| {
            let pid = proc.get("pid").and_then(Value::as_u64).map(|p| p as u32);
            let argv = proc
                .get("argv")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let cwd = proc
                .get("cwd")
                .and_then(Value::as_str)
                .or(fallback_cwd)
                .map(str::to_owned);

            ProcessInfo { pid, argv, cwd }
        })
        .collect()
}

/// `pane wait-output`: exit 0 means matched; exit 1 emits JSON on stderr for
/// *both* a timeout and a genuine server error (S2). Distinguishing the two
/// from that stderr JSON is UNVERIFIED — empirically verify against a pane
/// that closes mid-wait before shipping.
pub fn stderr_indicates_timeout(stderr_json: &Value) -> bool {
    const TIMEOUT_HINT_KEYS: [&str; 2] = ["timeout", "timed_out"];
    for key in TIMEOUT_HINT_KEYS {
        if let Some(v) = stderr_json.get(key)
            && v.as_bool() == Some(true)
        {
            return true;
        }
    }
    stderr_json
        .get("reason")
        .and_then(Value::as_str)
        .map(|r| r.eq_ignore_ascii_case("timeout"))
        .unwrap_or(false)
}
