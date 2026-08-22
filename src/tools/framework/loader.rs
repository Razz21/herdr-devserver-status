//! Directory-based spec loading. Runs once at daemon startup.
//!
//! Defaults are embedded via `include_str!` so they ship inside the
//! already-checksummed release binary instead of as loose files needing a
//! separate integrity check. Seeding here is a plain write-if-absent.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::log::{log, log_error, log_warn};

use super::FrameworkDetector;
use super::spec::FrameworkSpec;
use super::validate::validate;

const DEFAULT_SPECS: &[(&str, &str)] = &[
    ("vite.yml", include_str!("../../../frameworks/vite.yml")),
    ("nextjs.yml", include_str!("../../../frameworks/nextjs.yml")),
    ("nuxt.yml", include_str!("../../../frameworks/nuxt.yml")),
    ("astro.yml", include_str!("../../../frameworks/astro.yml")),
];

const SEED_STATE_FILE: &str = ".seed-state.json";

/// filename -> sha256 hex of the content this plugin last wrote for that
/// file. Distinguishes "untouched since we seeded it" from "user edited
/// it" when deciding whether to upgrade to a new bundled default.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SeedState {
    hashes: std::collections::BTreeMap<String, String>,
}

fn sha256_hex(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    format!("{:x}", hasher.finalize())
}

fn load_seed_state(frameworks_dir: &Path) -> SeedState {
    fs::read_to_string(frameworks_dir.join(SEED_STATE_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

fn save_seed_state(frameworks_dir: &Path, state: &SeedState) {
    let path = frameworks_dir.join(SEED_STATE_FILE);
    match serde_json::to_string_pretty(state) {
        Ok(json) => {
            if let Err(err) = fs::write(&path, json) {
                log_error(
                    &format!("frameworks: failed to write {}", path.display()),
                    &err,
                );
            }
        }
        Err(err) => log_error("frameworks: failed to serialize seed state", &err),
    }
}

/// Writes/upgrades each embedded default in `frameworks_dir`. Per file:
///
/// - missing: write it, record its hash.
/// - present, hash matches our last-recorded hash: untouched since we
///   wrote it — safe to upgrade to the new embedded default.
/// - present, hash doesn't match (including: no recorded hash at all):
///   user-owned. Never touch it, just log that a newer default exists.
///
/// Never overwrites blind — a file this plugin has never fingerprinted is
/// always treated as user-owned, even if it happens to equal an old
/// bundled default byte-for-byte.
fn seed_defaults(frameworks_dir: &Path) {
    if let Err(err) = fs::create_dir_all(frameworks_dir) {
        log_error(
            &format!(
                "frameworks: could not create {}, skipping seeding",
                frameworks_dir.display()
            ),
            &err,
        );
        return;
    }

    let mut state = load_seed_state(frameworks_dir);

    for (filename, contents) in DEFAULT_SPECS {
        let path = frameworks_dir.join(filename);
        let new_hash = sha256_hex(contents);

        if !path.exists() {
            if let Err(err) = fs::write(&path, contents) {
                log_error(
                    &format!("frameworks: failed to seed {}", path.display()),
                    &err,
                );
                continue;
            }
            state.hashes.insert((*filename).to_owned(), new_hash);
            continue;
        }

        let current = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(err) => {
                log_error(
                    &format!(
                        "frameworks: could not read {} to check for upgrade",
                        path.display()
                    ),
                    &err,
                );
                continue;
            }
        };
        let current_hash = sha256_hex(&current);

        if current_hash == new_hash {
            state.hashes.insert((*filename).to_owned(), new_hash);
            continue;
        }

        match state.hashes.get(*filename) {
            Some(last) if *last == current_hash => {
                if let Err(err) = fs::write(&path, contents) {
                    log_error(
                        &format!("frameworks: failed to upgrade {}", path.display()),
                        &err,
                    );
                    continue;
                }
                log(&format!(
                    "frameworks: upgraded {filename} to new bundled default"
                ));
                state.hashes.insert((*filename).to_owned(), new_hash);
            }
            _ => log(&format!(
                "frameworks: {filename} differs from bundled default and appears user-modified (or predates upgrade-tracking); leaving as-is"
            )),
        }
    }

    save_seed_state(frameworks_dir, &state);
}
/// Outcome of validating one file, shared by `load_all` (daemon startup)
/// and `validate_report` (the stateless `validate-specs` action).
enum FileOutcome {
    Accepted(Box<FrameworkSpec>),
    Rejected(String),
}

/// Directory read + sort + per-file `validate()` + check-8 (agent_name /
/// source uniqueness) collision resolution. Everything both callers need
/// in common; `load_all` additionally seeds first and builds detectors
/// after, `validate_report` does neither.
fn scan(frameworks_dir: &Path) -> Vec<(String, FileOutcome)> {
    let mut entries: Vec<_> = match fs::read_dir(frameworks_dir) {
        Ok(read_dir) => read_dir
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| {
                matches!(
                    p.extension().and_then(|e| e.to_str()),
                    Some("yml") | Some("yaml")
                )
            })
            .collect(),
        Err(_) => Vec::new(),
    };
    // Deterministic order — required for reproducible collision
    // resolution: first file in sorted filename order wins (check 8).
    entries.sort();

    let mut results = Vec::with_capacity(entries.len());
    // agent_name/source -> filename that first claimed it, for check-8
    // rejection messages and to detect the collision itself.
    let mut owner_by_agent_name: HashMap<String, String> = HashMap::new();
    let mut owner_by_source: HashMap<String, String> = HashMap::new();

    for path in &entries {
        let filename = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();

        let raw = match fs::read_to_string(path) {
            Ok(raw) => raw,
            Err(err) => {
                results.push((
                    filename,
                    FileOutcome::Rejected(format!("could not read file: {err}")),
                ));
                continue;
            }
        };

        let spec = match validate(&raw) {
            Ok(spec) => spec,
            Err(reason) => {
                results.push((filename, FileOutcome::Rejected(reason)));
                continue;
            }
        };

        if let Some(winner) = owner_by_agent_name
            .get(&spec.agent_name)
            .or_else(|| owner_by_source.get(&spec.source))
        {
            let reason = format!("duplicate agent_name/source, first-loaded wins: {winner}");
            results.push((filename, FileOutcome::Rejected(reason)));
            continue;
        }

        owner_by_agent_name.insert(spec.agent_name.clone(), filename.clone());
        owner_by_source.insert(spec.source.clone(), filename.clone());
        results.push((filename, FileOutcome::Accepted(Box::new(spec))));
    }

    results
}

/// Reads, seeds, validates, and builds every `FrameworkDetector` from
/// `config_dir/frameworks/*.{yml,yaml}`. Never panics and never blocks
/// daemon startup — a spec that fails to load is logged and skipped; the
/// daemon starts with whatever specs passed.
pub fn load_all(config_dir: &Path) -> Vec<FrameworkDetector> {
    let frameworks_dir = config_dir.join("frameworks");
    seed_defaults(&frameworks_dir);

    let results = scan(&frameworks_dir);
    let mut rejected_count = 0usize;
    let mut detectors = Vec::with_capacity(results.len());

    for (filename, outcome) in results {
        match outcome {
            FileOutcome::Rejected(reason) => {
                log_warn(&format!("frameworks: rejected {filename}: {reason}"));
                rejected_count += 1;
            }
            FileOutcome::Accepted(spec) => match FrameworkDetector::new(*spec) {
                Ok(detector) => detectors.push(detector),
                Err(reason) => {
                    log_warn(&format!("frameworks: rejected {filename}: {reason}"));
                    rejected_count += 1;
                }
            },
        }
    }

    log(&format!(
        "frameworks: loaded {}, rejected {} (see log for details)",
        detectors.len(),
        rejected_count
    ));

    detectors
}

/// Stateless dry-run for the `validate-specs` plugin action: same
/// directory scan + validation as `load_all`, minus seeding and detector
/// construction. Prints `OK <file>` / `REJECTED <file>: <reason>` per file
/// to stdout. Returns `true` iff nothing was rejected — the caller maps
/// that straight to the process exit code.
pub fn validate_report(frameworks_dir: &Path) -> bool {
    let results = scan(frameworks_dir);

    if results.is_empty() {
        println!(
            "no framework spec files found in {}",
            frameworks_dir.display()
        );
        return false;
    }

    let mut all_ok = true;
    for (filename, outcome) in &results {
        match outcome {
            FileOutcome::Accepted(_) => println!("OK {filename}"),
            FileOutcome::Rejected(reason) => {
                println!("REJECTED {filename}: {reason}");
                all_ok = false;
            }
        }
    }
    all_ok
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    const VALID_A: &str = r#"
schema_version: 1
source: "custom:one"
agent_name: "one"
display_agent: "One"
starting_message: "starting"
state_labels:
  idle: "idle"
  working: "working"
  blocked: "blocked"
bin_path_pattern: '(^|/)one\.js$'
package_name: "one"
signals:
  - kind: ready
    needle: "ready"
    counts_as_recent_success: true
"#;

    // Same agent_name as VALID_A ("one") — must lose the collision to
    // whichever file sorts first.
    const VALID_B_DUPLICATE_AGENT: &str = r#"
schema_version: 1
source: "custom:two"
agent_name: "one"
display_agent: "Two"
starting_message: "starting"
state_labels:
  idle: "idle"
  working: "working"
  blocked: "blocked"
bin_path_pattern: '(^|/)two\.js$'
package_name: "two"
signals:
  - kind: ready
    needle: "ready"
    counts_as_recent_success: true
"#;

    const INVALID_EMPTY_SIGNALS: &str = r#"
schema_version: 1
source: "custom:three"
agent_name: "three"
display_agent: "Three"
starting_message: "starting"
state_labels:
  idle: "idle"
  working: "working"
  blocked: "blocked"
bin_path_pattern: '(^|/)three\.js$'
package_name: "three"
signals: []
"#;

    /// Unique per-test scratch dir under the OS temp dir, so tests don't
    /// need a `tempfile` dependency. A leftover dir from a crashed run is
    /// harmless — next run gets a fresh path.
    fn scratch_dir(test_name: &str) -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "herdr-devserver-status-loader-test-{test_name}-{}-{n}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("create scratch dir");
        dir
    }

    #[test]
    fn load_all_seeds_defaults_into_empty_dir() {
        let config_dir = scratch_dir("seed");
        let detectors = load_all(&config_dir);
        // The four bundled defaults (vite/nextjs/nuxt/astro) all validate,
        // so seeding + loading an empty dir should produce exactly four.
        assert_eq!(detectors.len(), 4);
        for name in ["vite.yml", "nextjs.yml", "nuxt.yml", "astro.yml"] {
            assert!(
                config_dir.join("frameworks").join(name).is_file(),
                "expected seeded file: {name}"
            );
        }
        fs::remove_dir_all(&config_dir).ok();
    }

    #[test]
    fn load_all_never_overwrites_an_existing_file() {
        let config_dir = scratch_dir("no-overwrite");
        let frameworks_dir = config_dir.join("frameworks");
        fs::create_dir_all(&frameworks_dir).unwrap();
        fs::write(frameworks_dir.join("vite.yml"), "not valid yaml: [").unwrap();

        let detectors = load_all(&config_dir);
        // vite.yml was user-edited garbage and must NOT have been
        // overwritten by seeding — it stays rejected, other 3 defaults
        // still seed and load fine.
        assert_eq!(detectors.len(), 3);
        assert!(!detectors.iter().any(|d| d.spec.agent_name == "vite"));
        let contents = fs::read_to_string(frameworks_dir.join("vite.yml")).unwrap();
        assert_eq!(contents, "not valid yaml: [");

        fs::remove_dir_all(&config_dir).ok();
    }

    #[test]
    fn duplicate_agent_name_first_file_wins() {
        let config_dir = scratch_dir("dup");
        let frameworks_dir = config_dir.join("frameworks");
        fs::create_dir_all(&frameworks_dir).unwrap();
        fs::write(frameworks_dir.join("a-one.yml"), VALID_A).unwrap();
        fs::write(frameworks_dir.join("b-one.yml"), VALID_B_DUPLICATE_AGENT).unwrap();

        let results = scan(&frameworks_dir);
        let outcomes: Vec<_> = results
            .iter()
            .map(|(name, outcome)| (name.as_str(), matches!(outcome, FileOutcome::Accepted(_))))
            .collect();
        // Sorted order: "a-one.yml" < "b-one.yml", so a-one wins.
        assert_eq!(outcomes, vec![("a-one.yml", true), ("b-one.yml", false)]);

        fs::remove_dir_all(&config_dir).ok();
    }

    #[test]
    fn invalid_file_is_reported_and_skipped() {
        let config_dir = scratch_dir("invalid");
        let frameworks_dir = config_dir.join("frameworks");
        fs::create_dir_all(&frameworks_dir).unwrap();
        fs::write(frameworks_dir.join("three.yml"), INVALID_EMPTY_SIGNALS).unwrap();

        let results = scan(&frameworks_dir);
        assert_eq!(results.len(), 1);
        match &results[0].1 {
            FileOutcome::Rejected(reason) => {
                assert_eq!(reason, "signals empty or contains empty needle")
            }
            FileOutcome::Accepted(_) => panic!("expected rejection"),
        }

        fs::remove_dir_all(&config_dir).ok();
    }

    #[test]
    fn validate_report_exit_semantics() {
        let config_dir = scratch_dir("validate-report");
        let frameworks_dir = config_dir.join("frameworks");
        fs::create_dir_all(&frameworks_dir).unwrap();
        fs::write(frameworks_dir.join("one.yml"), VALID_A).unwrap();
        assert!(validate_report(&frameworks_dir));

        fs::write(frameworks_dir.join("three.yml"), INVALID_EMPTY_SIGNALS).unwrap();
        assert!(!validate_report(&frameworks_dir));

        // Never seeds — only the two files written above should exist.
        let count = fs::read_dir(&frameworks_dir).unwrap().count();
        assert_eq!(count, 2);

        fs::remove_dir_all(&config_dir).ok();
    }
}
