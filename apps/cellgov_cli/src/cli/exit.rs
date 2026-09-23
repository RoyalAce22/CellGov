//! Command diagnostics and process-status handling.

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CommandExitCode(u8);

impl CommandExitCode {
    pub(crate) const SUCCESS: Self = Self(0);

    /// Accepts exit codes in the portable `u8` range.
    ///
    /// # Panics
    ///
    /// Panics if `code` is outside the portable `u8` status range.
    pub(crate) const fn new(code: i32) -> Self {
        assert!(
            code >= 0 && code <= u8::MAX as i32,
            "exit status must fit in u8"
        );
        Self(code as u8)
    }

    pub(crate) const fn value(self) -> u8 {
        self.0
    }
}

/// An owned diagnostic that the process boundary reports.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub(crate) struct Diagnostic(Box<str>);

impl Diagnostic {
    /// Capture a command diagnostic without printing or terminating.
    pub(crate) fn new(message: impl Into<Box<str>>) -> Self {
        Self(message.into())
    }
}

/// A command failure for the single process boundary in `main`.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CommandError {
    /// Uses the shared failed-operation status.
    #[error("{0}")]
    Failed(#[source] Diagnostic),

    #[error(transparent)]
    Fuzz(#[from] super::fuzz::FuzzCliError),

    #[error("{diagnostic}")]
    Status {
        code: CommandExitCode,
        #[source]
        diagnostic: Diagnostic,
    },

    /// A child ended from Ctrl-C, which the process boundary must reproduce.
    #[error("command interrupted")]
    Interrupted,
}

impl CommandError {
    pub(crate) fn failed(message: impl Into<Box<str>>) -> Self {
        Self::Failed(Diagnostic::new(message))
    }

    pub(crate) fn status(code: i32, message: impl Into<Box<str>>) -> Self {
        Self::Status {
            code: CommandExitCode::new(code),
            diagnostic: Diagnostic::new(message),
        }
    }

    /// Returns no status for Ctrl-C.
    pub(crate) fn code(&self) -> Option<CommandExitCode> {
        match self {
            Self::Failed(_) => Some(CommandExitCode::new(super::exit_codes::FAILED)),
            Self::Fuzz(error) => Some(CommandExitCode::new(if error.is_broken_pipe() {
                super::exit_codes::BROKEN_PIPE
            } else if error.is_usage() {
                super::exit_codes::USAGE
            } else {
                super::exit_codes::FAILED
            })),
            Self::Status { code, .. } => Some(*code),
            Self::Interrupted => None,
        }
    }
}

/// Reports a command error at the process boundary.
///
/// `None` tells `main` to reproduce a child's Ctrl-C without a diagnostic.
pub(crate) fn report(error: CommandError) -> Option<CommandExitCode> {
    if matches!(error, CommandError::Interrupted) {
        return None;
    }
    cellgov_terminal::progress::release_terminal();
    eprintln!("{error}");
    error.code()
}

/// Propagate a child's Ctrl-C termination to the process boundary.
pub(crate) fn propagate_interrupt(child: std::process::ExitStatus) -> Result<(), CommandError> {
    if cellgov_terminal::interrupt::was_interrupted(child) {
        return Err(CommandError::Interrupted);
    }
    Ok(())
}
