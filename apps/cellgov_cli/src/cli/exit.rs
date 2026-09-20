//! Process termination helpers for CLI commands.

/// Print `msg` to stderr and exit with the failed-operation status.
pub(crate) fn die(msg: &str) -> ! {
    die_with_status(msg, super::exit_codes::FAILED)
}

/// Release terminal state and exit with the failed-operation status.
pub(crate) fn exit_failed() -> ! {
    cellgov_terminal::progress::release_terminal();
    std::process::exit(super::exit_codes::FAILED)
}

/// End this process when `child` ended from an interrupt.
pub(crate) fn propagate_interrupt(child: std::process::ExitStatus) {
    if cellgov_terminal::interrupt::was_interrupted(child) {
        cellgov_terminal::interrupt::exit_interrupted();
    }
}

/// [`die`] with the status the caller names.
pub(crate) fn die_with_status(msg: &str, status: i32) -> ! {
    cellgov_terminal::progress::release_terminal();
    eprintln!("{msg}");
    std::process::exit(status)
}
