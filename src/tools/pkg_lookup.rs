//! Shared "does this resolved script path actually belong to package X"
//! check. Walks upward from the script's directory to find the nearest
//! `package.json` and verifies its `name` field matches the expected package.

use std::fs;
use std::path::Path;

use serde::Deserialize;

#[derive(Deserialize)]
struct PackageJson {
    name: Option<String>,
}

pub struct PackageMatch {
    pub package_json_path: String,
}

/// Walks upward from `entry_path`'s parent directory for the nearest
/// `package.json`, up to `max_levels` directories. Matches only if that
/// nearest manifest's `name` equals `expected_name` exactly.
///
/// Stops at the first manifest found rather than skipping a non-matching
/// one to search further up — doing so risks matching an unrelated
/// ancestor manifest instead (e.g. a monorepo root's `package.json`).
pub fn find_owning_package(
    entry_path: &str,
    expected_name: &str,
    max_levels: usize,
) -> Option<PackageMatch> {
    let mut dir = Path::new(entry_path).parent()?;
    for _ in 0..max_levels {
        let candidate = dir.join("package.json");
        if candidate.is_file() {
            let name = fs::read_to_string(&candidate)
                .ok()
                .and_then(|raw| serde_json::from_str::<PackageJson>(&raw).ok())
                .and_then(|pkg| pkg.name);

            return if name.as_deref() == Some(expected_name) {
                Some(PackageMatch {
                    package_json_path: candidate.to_string_lossy().into_owned(),
                })
            } else {
                None
            };
        }
        dir = dir.parent()?;
    }
    None
}