//! Process-exit and whole-file-read helpers shared across every
//! CLI subcommand.
//!
//! `die` + `load_file_or_die` live in their own leaf module so
//! modules that want the helpers (arg parsing, scenario loaders,
//! command dispatchers) do not have to drag in unrelated CLI
//! machinery. This is the only module in `cli/` that nothing else
//! depends on semantically, so it sits at the bottom of the
//! dependency graph.

/// Print `msg` to stderr and exit the process with status 1.
pub(crate) fn die(msg: &str) -> ! {
    eprintln!("{msg}");
    std::process::exit(1)
}

/// Read the entire contents of a file or die with a context-rich
/// error message. Used wherever a missing or unreadable input is
/// a fatal configuration problem rather than a recoverable one.
pub(crate) fn load_file_or_die(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap_or_else(|e| die(&format!("failed to read {path}: {e}")))
}
