pub mod discovery;
pub mod worker;

/// Entry point for the `daemon` subcommand. Runs for the lifetime of the
/// Herdr session — if Herdr restarts, a fresh `[[startup]]` hook fires and
/// replaces this process. No restart-on-crash within a session if this
/// process itself dies (same caveat the original plan raised).
pub fn run() -> ! {
    discovery::run()
}
