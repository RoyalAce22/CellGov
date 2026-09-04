//! The exit statuses every `cellgov` command shares.
//!
//! [`CONTRACT`] is what `--help` prints. A command with an outcome the
//! contract does not cover gives it a code at or above
//! [`FIRST_COMMAND_SPECIFIC`] and names it in its own help.

/// The operation ran and failed.
pub(crate) const FAILED: i32 = 1;

/// The command line was wrong:
///
/// - an unknown flag,
/// - a missing required argument,
/// - a bad value,
/// - a confirmation the run could not ask.
pub(crate) const USAGE: i32 = 2;

/// Runs that had to reproduce each other disagreed.
pub(crate) const DISAGREED: i32 = 3;

/// A subprocess failed, or a verification found the store diverged from
/// its records.
pub(crate) const DIVERGED: i32 = 4;

/// A boot moved off its committed anchor.
pub(crate) const ANCHOR_MOVED: i32 = 5;

/// The lowest status a command may give an outcome of its own.
pub(crate) const FIRST_COMMAND_SPECIFIC: i32 = 10;

/// `code`, checked against the command-specific range.
///
/// Every status a single command defines goes through this function.
///
/// # Panics
///
/// Panics if `code` is below [`FIRST_COMMAND_SPECIFIC`]. A const caller
/// gets a compile error.
pub(crate) const fn command_specific(code: i32) -> i32 {
    assert!(
        code >= FIRST_COMMAND_SPECIFIC,
        "a command-specific exit status must not take a value the shared contract defines"
    );
    code
}

/// The contract as `--help` prints it.
pub(crate) const CONTRACT: &str = "\
Exit codes:
  0    success
  1    the operation ran and failed
  2    usage error
  3    runs that had to reproduce each other disagreed
  4    a subprocess failed, or a verification diverged
  5    a boot moved off its committed anchor
  >=10 an outcome particular to one command; its own help names it";

#[cfg(test)]
#[path = "tests/exit_codes_tests.rs"]
mod tests;
