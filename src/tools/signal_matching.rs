//! Generic output-signal matching engine. Scans for known substrings,
//! derives a status from which kinds were seen, and decides error state
//! from last-occurrence positions. Framework-agnostic; each framework
//! supplies its own `OutputSignal` table.

use regex::Regex;
use serde::Deserialize;

use crate::tools::ToolStatus;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    Starting,
    Ready,
    Error,
    #[allow(dead_code)] // no `recovered` needle exists yet for any tool; kept for completeness
    Recovered,
    Building,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OutputSignal {
    pub kind: SignalKind,
    /// Plain substring, not regex — each tool only needs a handful of
    /// known-stable CLI strings.
    pub needle: String,
    /// Whether `has_recent_error` treats this needle as a "recent
    /// success" marker when comparing last-error-vs-last-success
    /// position. Only meaningful for `SignalKind::Ready` needles.
    #[serde(default)]
    pub counts_as_recent_success: bool,
}

pub struct MatchResult {
    pub signals: Vec<SignalKind>,
    pub url: Option<String>,
    pub port: Option<u16>,
}

/// Scans `recent_output` for every needle, and — if `url_pattern` is
/// given — extracts the last URL/port match.
pub fn match_output(
    signals_table: &[OutputSignal],
    url_pattern: Option<&Regex>,
    recent_output: &str,
) -> MatchResult {
    let signals = signals_table
        .iter()
        .filter(|s| recent_output.contains(s.needle.as_str()))
        .map(|s| s.kind)
        .collect();

    let (url, port) = match url_pattern.and_then(|re| re.captures_iter(recent_output).last()) {
        Some(caps) => {
            let url = caps.get(1).map(|m| m.as_str().to_owned());
            let port = caps.get(2).and_then(|m| m.as_str().parse::<u16>().ok());
            (url, port)
        }
        None => (None, None),
    };

    MatchResult { signals, url, port }
}

/// Last occurrence of any error-kind needle vs. any needle flagged
/// `counts_as_recent_success` — true if the last error is after the last
/// success.
pub fn has_recent_error(signals_table: &[OutputSignal], output: &str) -> bool {
    let last_error = signals_table
        .iter()
        .filter(|s| s.kind == SignalKind::Error)
        .filter_map(|s| output.rfind(s.needle.as_str()))
        .max();
    let last_success = signals_table
        .iter()
        .filter(|s| s.counts_as_recent_success)
        .filter_map(|s| output.rfind(s.needle.as_str()))
        .max();

    match (last_error, last_success) {
        (Some(e), Some(s)) => e > s,
        (Some(_), None) => true,
        _ => false,
    }
}

pub struct DerivedStatus {
    pub status: ToolStatus,
    pub has_errors: bool,
}

pub fn derive_status(previous_had_errors: bool, result: &MatchResult) -> DerivedStatus {
    let is_running = result.signals.contains(&SignalKind::Ready) || result.url.is_some();
    let saw_error = result.signals.contains(&SignalKind::Error);

    if is_running {
        // Caller overrides with has_recent_error when both ready and error
        // are present in the same read.
        DerivedStatus {
            status: ToolStatus::Running,
            has_errors: false,
        }
    } else {
        DerivedStatus {
            status: ToolStatus::Starting,
            has_errors: if saw_error { true } else { previous_had_errors },
        }
    }
}

/// One alternation regex built from every needle, for `pane wait-output
/// --regex`. Triggering check only — `match_output` still runs the full
/// substring scan against the freshly-read output afterward.
pub fn build_signal_regex(signals_table: &[OutputSignal]) -> Regex {
    let alternation = signals_table
        .iter()
        .map(|s| regex::escape(&s.needle))
        .collect::<Vec<_>>()
        .join("|");
    Regex::new(&alternation).expect("alternation built from escaped literal needles is always a valid pattern")
}