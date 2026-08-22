use std::path::PathBuf;

use crate::log::log_warn;

/// `$HERDR_PLUGIN_CONFIG_DIR`, Herdr-provided and user-editable.
/// Falls back to the current directory if unset so a
/// run still does something (with defaults seeded there) rather than
/// refusing outright — Herdr is expected to always set this for a plugin
/// process, so the fallback is a safety net, not the expected path.
pub fn plugin_config_dir() -> PathBuf {
    match std::env::var("HERDR_PLUGIN_CONFIG_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => {
            log_warn("HERDR_PLUGIN_CONFIG_DIR not set, falling back to current directory");
            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
        }
    }
}