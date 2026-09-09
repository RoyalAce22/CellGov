//! Ends a process the way the default Ctrl-C action would, and tells a
//! parent that its child ended that way.
//!
//! A shell `for` loop breaks on a child that died by SIGINT. It
//! continues past a child that exited 130, so one Ctrl-C would leave
//! it to run every remaining iteration. On Unix the process therefore
//! resets the disposition to the default and re-raises the signal. On
//! Windows the process exits with `STATUS_CONTROL_C_EXIT`, the status
//! the default console handler gives, and the console delivers the
//! event to the shell as well.

use std::process::ExitStatus;

/// The status of a process the default Ctrl-C action ended on
/// Windows: `STATUS_CONTROL_C_EXIT`.
#[cfg(windows)]
const CONTROL_C_EXIT: u32 = 0xC000_013A;

/// Restore the terminal, then end the process as the default Ctrl-C
/// action would.
///
/// On Unix the process resets the SIGINT disposition to the default
/// and re-raises the signal, so it dies by SIGINT. On Windows it exits
/// with `STATUS_CONTROL_C_EXIT`.
///
/// The Ctrl-C handler calls this, and so does a parent whose child
/// [`was_interrupted`]: the parent ends the same way, so the interrupt
/// reaches whatever drives the parent.
pub fn exit_interrupted() -> ! {
    crate::progress::release_terminal();
    #[cfg(unix)]
    {
        use std::io::Write as _;
        // signal-hook `low_level::emulate_default_handler`: for a
        // terminating signal it returns only when the signal is not
        // in its table. A reset or raise that fails aborts the process,
        // and a raise that works never returns.
        let raised = signal_hook::low_level::emulate_default_handler(signal_hook::consts::SIGINT);
        let status = 128 + signal_hook::consts::SIGINT;
        if let Err(e) = raised {
            let _ = writeln!(
                std::io::stderr(),
                "cellgov: SIGINT could not be re-raised ({e}); exiting {status}"
            );
        }
        std::process::exit(status)
    }
    #[cfg(windows)]
    {
        std::process::exit(CONTROL_C_EXIT as i32)
    }
}

/// Whether a child ended the way a Ctrl-C ends one: killed by SIGINT
/// on Unix, exited `STATUS_CONTROL_C_EXIT` on Windows.
///
/// A Unix child that exited 130 is not one: it chose that status
/// itself, and its parent has no interrupt to propagate.
pub fn was_interrupted(status: ExitStatus) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt as _;
        status.signal() == Some(signal_hook::consts::SIGINT)
    }
    #[cfg(windows)]
    {
        status.code() == Some(CONTROL_C_EXIT as i32)
    }
}

#[cfg(test)]
#[path = "tests/interrupt_status_tests.rs"]
mod tests;
