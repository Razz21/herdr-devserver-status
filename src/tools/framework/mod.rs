pub mod loader;
pub mod spec;
pub mod validate;

use std::fs;

use regex::Regex;

use crate::herdr::wire::ProcessInfo;
use crate::tools::pkg_lookup::find_owning_package;
use crate::tools::signal_matching::{self, SignalKind};
use crate::tools::{ProcessMatch, ToolDetector, ToolMatchResult, ToolStatus};
use spec::FrameworkSpec;

/// `ToolDetector` impl shared by every data-driven `FrameworkSpec`.
/// Compiles the spec's regexes once at construction, not per poll tick.
pub struct FrameworkDetector {
    spec: FrameworkSpec,
    bin_re: Regex,
    signal_re: Regex,
    url_re: Option<Regex>,
}

impl FrameworkDetector {
    pub fn new(spec: FrameworkSpec) -> Result<Self, String> {
        let bin_re = Regex::new(&spec.bin_path_pattern).map_err(|e| {
            format!(
                "bad bin_path_pattern for framework spec {:?}: {e}",
                spec.agent_name
            )
        })?;
        let signal_re = signal_matching::build_signal_regex(&spec.signals);
        let url_re = match &spec.url_pattern {
            Some(pattern) => Some(Regex::new(pattern).map_err(|e| {
                format!(
                    "bad url_pattern for framework spec {:?}: {e}",
                    spec.agent_name
                )
            })?),
            None => None,
        };

        Ok(Self {
            spec,
            bin_re,
            signal_re,
            url_re,
        })
    }
}

impl ToolDetector for FrameworkDetector {
    fn source(&self) -> &str {
        &self.spec.source
    }

    fn agent_name(&self) -> &str {
        &self.spec.agent_name
    }

    fn display_agent(&self) -> &str {
        &self.spec.display_agent
    }

    fn starting_message(&self) -> &str {
        &self.spec.starting_message
    }

    fn state_labels(&self) -> &[(String, String)] {
        &self.spec.state_labels
    }

    fn clear_token_names(&self) -> &[String] {
        &self.spec.clear_token_names
    }

    /// Resolve each argv entry, match it against the spec's bin-path pattern,
    /// then confirm the resolved script is owned by the expected npm package.
    fn confirm(&self, procs: &[ProcessInfo]) -> Option<ProcessMatch> {
        for proc in procs {
            let Some(pid) = proc.pid else { continue };

            for candidate in &proc.argv {
                let real = fs::canonicalize(candidate)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_else(|_| candidate.clone());

                if !self.bin_re.is_match(&real) {
                    continue;
                }

                let Some(pkg_match) = find_owning_package(&real, &self.spec.package_name, 6)
                else {
                    continue;
                };

                return Some(ProcessMatch {
                    pid,
                    cwd: proc.cwd.clone(),
                    details: vec![
                        ("entry".to_owned(), real),
                        ("package_json".to_owned(), pkg_match.package_json_path),
                    ],
                });
            }
        }
        None
    }

    fn signal_regex(&self) -> &Regex {
        &self.signal_re
    }

    fn match_output(&self, output: &str, previous_had_errors: bool) -> ToolMatchResult {
        let result = signal_matching::match_output(&self.spec.signals, self.url_re.as_ref(), output);
        let derived = signal_matching::derive_status(previous_had_errors, &result);

        let has_errors = if derived.status == ToolStatus::Running
            && result.signals.contains(&SignalKind::Ready)
            && result.signals.contains(&SignalKind::Error)
        {
            signal_matching::has_recent_error(&self.spec.signals, output)
        } else {
            derived.has_errors
        };

        let is_building = result.signals.contains(&SignalKind::Building)
            && !result.signals.contains(&SignalKind::Ready);

        let mut extra_tokens = vec![(
            format!("{}_has_errors", self.spec.agent_name),
            has_errors.to_string(),
        )];
        if let Some(url) = &result.url {
            extra_tokens.push((format!("{}_url", self.spec.agent_name), url.clone()));
        }
        if let Some(port) = result.port {
            extra_tokens.push((format!("{}_port", self.spec.agent_name), port.to_string()));
        }

        ToolMatchResult {
            status: derived.status,
            has_errors,
            is_building,
            extra_tokens,
        }
    }
}