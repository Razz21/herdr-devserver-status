//! Pluggable tool detection. Add a new detected tool either by
//! implementing `ToolDetector` directly (needed when a framework's process
//! confirmation can't be expressed as "one bin-path regex + one owning
//! package.json name"), or, for the common case, by dropping a
//! `<agent_name>.yml` file into `$HERDR_PLUGIN_CONFIG_DIR/frameworks/`
//! and letting `framework::loader` + `framework::FrameworkDetector` handle
//! it generically. Either way, nothing in
//! `daemon/discovery.rs` or `daemon/worker.rs` needs to change.

pub mod framework;
pub mod pkg_lookup;
pub mod signal_matching;

use std::path::Path;

use regex::Regex;

use crate::herdr::wire::ProcessInfo;

/// Confirmed match of a tracked tool's process within a pane.
pub struct ProcessMatch {
    pub pid: u32,
    pub cwd: Option<String>,
    /// Extra key/value pairs for the "CONFIRMED" discovery log line —
    /// detector-specific diagnostic detail (resolved script path,
    /// package.json path). Logging only, never sent to Herdr.
    pub details: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolStatus {
    Starting,
    Running,
}

/// Result of matching one read of pane output against a detector's signals.
pub struct ToolMatchResult {
    pub status: ToolStatus,
    pub has_errors: bool,
    pub is_building: bool,
    /// Every metadata token this detector wants reported for this read —
    /// fully named by the detector itself (e.g. `"vite_has_errors"`,
    /// `"nextjs_url"`). Reported verbatim; worker.rs does not synthesize or
    /// prefix these.
    pub extra_tokens: Vec<(String, String)>,
}

/// One tool this daemon can detect and track inside a Herdr pane.
///
/// `source`/`agent_name` must be unique per detector so two tools tracked
/// concurrently never collide. `confirm` matches on the resolved
/// script/binary being run, not on what launched it — node/bun/etc show
/// up differently in argv[0].
pub trait ToolDetector: Send + Sync {
    fn source(&self) -> &str;
    fn agent_name(&self) -> &str;
    fn display_agent(&self) -> &str;
    fn starting_message(&self) -> &str;
    fn state_labels(&self) -> &[(String, String)];
    fn clear_token_names(&self) -> &[String];

    fn confirm(&self, procs: &[ProcessInfo]) -> Option<ProcessMatch>;

    fn signal_regex(&self) -> &Regex;
    fn match_output(
        &self,
        output: &str,
        previous_status: ToolStatus,
        previous_had_errors: bool,
    ) -> ToolMatchResult;
}

/// All detectors tried, in order, against each newly seen pane's process
/// list. First match wins. `config_dir` is `$HERDR_PLUGIN_CONFIG_DIR`,
/// resolved by the caller so this stays testable against any directory.
pub fn all_detectors(config_dir: &Path) -> Vec<Box<dyn ToolDetector>> {
    framework::loader::load_all(config_dir)
        .into_iter()
        .map(|d| Box::new(d) as Box<dyn ToolDetector>)
        .collect()
}
