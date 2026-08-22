use serde::Deserialize;

use crate::tools::signal_matching::OutputSignal;

/// One framework's dev-server detection config, loaded from YAML at
/// daemon startup. `FrameworkDetector` implements the shared "one
/// bin-path regex + one owning package.json name" confirmation strategy
/// plus the common output-matching engine on top of whatever a spec
/// declares.
///
/// Only fits a framework whose dev server is a single long-lived process
/// resolvable to one script path — not one that forks a native child
/// process to do the real work, since bin-path regex has nothing to match
/// in the child (see the `nextjs.yml` note on Turbopack).
#[derive(Debug, Clone)]
pub struct FrameworkSpec {
    pub source: String,
    pub agent_name: String,
    pub display_agent: String,
    pub starting_message: String,
    /// Always [idle, working, blocked], in that order — enforced by
    /// `YamlStateLabels`'s fixed field set, not by this type.
    pub state_labels: Vec<(String, String)>,
    pub clear_token_names: Vec<String>,

    /// Regex fragment matched against each canonicalized argv entry.
    pub bin_path_pattern: String,

    /// Expected `name` field of the package.json owning the resolved bin
    /// path (checked via `pkg_lookup::find_owning_package`), e.g. `"next"`.
    pub package_name: String,

    pub signals: Vec<OutputSignal>,

    /// Regex with two capture groups: (full URL, port). `None` if this
    /// framework's dev server output isn't worth extracting a URL from.
    pub url_pattern: Option<String>,
}

/// On-disk YAML shape, kept separate from `FrameworkSpec` so the wire
/// format and the runtime representation can evolve independently.
///
/// `deny_unknown_fields` is check 1 of the loader's validation: an unknown
/// key is a hard parse error, not silently ignored.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YamlFrameworkSpec {
    pub schema_version: u32,
    pub source: String,
    pub agent_name: String,
    pub display_agent: String,
    pub starting_message: String,
    pub state_labels: YamlStateLabels,
    #[serde(default)]
    pub clear_token_names: Vec<String>,
    pub bin_path_pattern: String,
    pub package_name: String,
    pub signals: Vec<OutputSignal>,
    #[serde(default)]
    pub url_pattern: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct YamlStateLabels {
    pub idle: String,
    pub working: String,
    pub blocked: String,
}

/// Newest schema_version this loader accepts (check 2). Mismatches are
/// rejected outright, never guessed at or migrated.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

impl From<YamlFrameworkSpec> for FrameworkSpec {
    /// `schema_version` has no runtime use past the loader's validation
    /// gate (check 2), so it's dropped here.
    fn from(y: YamlFrameworkSpec) -> Self {
        FrameworkSpec {
            source: y.source,
            agent_name: y.agent_name,
            display_agent: y.display_agent,
            starting_message: y.starting_message,
            state_labels: vec![
                ("idle".to_owned(), y.state_labels.idle),
                ("working".to_owned(), y.state_labels.working),
                ("blocked".to_owned(), y.state_labels.blocked),
            ],
            clear_token_names: y.clear_token_names,
            bin_path_pattern: y.bin_path_pattern,
            package_name: y.package_name,
            signals: y.signals,
            url_pattern: y.url_pattern,
        }
    }
}