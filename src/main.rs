mod config;
mod daemon;
mod herdr;
mod log;
mod tools;

use config::plugin_config_dir;

#[cfg(debug_assertions)]
fn load_dev_env() {
    match dotenvy::dotenv() {
        Ok(_) | Err(dotenvy::Error::Io(_)) => {
            // Io(_) here means no .env file — expected, not an error.
        }
        Err(err) => eprintln!("[herdr-devserver-status] .env found but failed to parse: {err}"),
    }
}

#[cfg(not(debug_assertions))]
fn load_dev_env() {}

fn main() {
    load_dev_env();

    let mut args = std::env::args().skip(1);
    let command = args.next();

    match command.as_deref() {
        Some("daemon") => daemon::run(),
        Some("validate-specs") => validate_specs(args.next()),
        other => {
            eprintln!(
                "[herdr-devserver-status] unknown command: {}",
                other.unwrap_or("(none)")
            );
            std::process::exit(1);
        }
    }
}

/// Validates every `*.yml`/`*.yaml` in `dir_override`, else
/// `$HERDR_PLUGIN_CONFIG_DIR/frameworks`. Exit 0 iff every file validated
/// and at least one was found.
fn validate_specs(dir_override: Option<String>) {
    let frameworks_dir = match dir_override {
        Some(dir) => std::path::PathBuf::from(dir),
        None => plugin_config_dir().join("frameworks"),
    };
    let all_ok = tools::framework::loader::validate_report(&frameworks_dir);
    std::process::exit(if all_ok { 0 } else { 1 });
}
