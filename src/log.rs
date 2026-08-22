use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::LazyLock;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
}

impl fmt::Display for LogLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            LogLevel::Error => "ERROR",
            LogLevel::Warn => "WARN",
            LogLevel::Info => "INFO",
            LogLevel::Debug => "DEBUG",
        })
    }
}

fn resolve_level() -> LogLevel {
    match std::env::var("HDS_LOG_LEVEL")
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "error" => LogLevel::Error,
        "warn" | "warning" => LogLevel::Warn,
        "debug" => LogLevel::Debug,
        _ => LogLevel::Info,
    }
}

static LOG_LEVEL: LazyLock<LogLevel> = LazyLock::new(resolve_level);

fn default_log_path() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("HDS_LOG_PATH") {
        return Some(PathBuf::from(path));
    }
    std::env::var("HERDR_PLUGIN_CONFIG_DIR")
        .ok()
        .map(|base| PathBuf::from(base).join("herdr-devserver-status.log"))
}

static LOG_PATH: LazyLock<Option<PathBuf>> = LazyLock::new(default_log_path);

fn timestamp() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_else(|_| String::from("unknown-time"))
}

fn write_log(level: LogLevel, msg: &str) {
    if level > *LOG_LEVEL {
        return;
    }
    let line = format!("[{}] [{level}] {msg}", timestamp());
    eprintln!("{line}");
    let Some(path) = LOG_PATH.as_ref() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

pub fn log(msg: &str) {
    write_log(LogLevel::Info, msg);
}

pub fn log_error(context: &str, err: &dyn std::error::Error) {
    write_log(LogLevel::Error, &format!("{context}: {err}"));
}

pub fn log_warn(msg: &str) {
    write_log(LogLevel::Warn, msg);
}

pub fn log_debug(msg: &str) {
    write_log(LogLevel::Debug, msg);
}
