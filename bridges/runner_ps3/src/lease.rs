//! One runner per console at a time: a lease held on the console
//! itself, so every runner that reaches the console sees it, whatever
//! machine, account or host spelling it runs under.
//!
//! The lease is the directory [`LEASE_DIR`]. Taking it is FTP `MKD`,
//! which webMAN refuses with `550` for a directory that exists, so
//! creation is the atomic step; the [`HOLDER_FILE`] stored into it then
//! names who holds the console. A refusal reads that file and names the
//! command that clears it. Releasing the lease removes the file and the
//! directory; `unlock` does the same whoever holds it.
//!
//! The runner cannot reach the console from a destructor, so a run that
//! panics leaves its lease behind, and `unlock` clears it. The runner
//! ignores the lease files earlier runners kept in the host's temp
//! directory.

use crate::run::{ConsoleOps, RESULT_ROOT};
use crate::transcript::Transcript;
use crate::transport::TransportError;

/// The lease directory's name under [`RESULT_ROOT`].
pub const LEASE_NAME: &str = "cellgov_lease";

/// The lease directory on the console.
pub const LEASE_DIR: &str = "/dev_hdd0/tmp/cellgov_lease";

/// The file in [`LEASE_DIR`] that names the holder.
pub const HOLDER_FILE: &str = "/dev_hdd0/tmp/cellgov_lease/holder";

/// The FTP reply webMAN gives `MKD` for a directory that exists.
const EXISTS: u16 = 550;

/// A held lease on one console.
#[derive(Debug)]
#[must_use = "a lease is released with `release`, or it holds the console until `unlock`"]
pub struct Lease {
    _held: (),
}

/// Why a lease was not taken or not released.
#[derive(Debug, thiserror::Error)]
pub enum LeaseError {
    /// Another runner, or a run that did not release, holds the console.
    #[error("{LEASE_DIR} on {host} holds the console for {holder}; clear it with `{unlock_with}`")]
    Held {
        /// The console.
        host: String,
        /// What the holder file names, or why there is none.
        holder: String,
        /// The `unlock` command that clears it, through the front end
        /// that asked for the lease.
        unlock_with: String,
    },
    /// The console did not take, show or remove the lease.
    #[error("{LEASE_DIR}: {0}")]
    Transport(#[from] TransportError),
}

/// What a holder file says: who runs the runner, on which machine,
/// under which process, for which microtest, through which front end.
pub fn holder_text(who: &str, test: &str, invocation: &str) -> String {
    format!(
        "{who}, pid {}, test {test}, via {invocation}\n",
        std::process::id()
    )
}

impl Lease {
    /// Take the lease on `console` (at `host`) for `holder`, the text a
    /// refusal shows another runner. `invocation` is the front end's
    /// command, such as `runner_ps3`, which a refusal's `unlock` remedy
    /// starts with.
    ///
    /// # Errors
    ///
    /// [`LeaseError::Held`] when the directory exists, naming its holder,
    /// and [`LeaseError::Transport`] for any other failure. When the
    /// holder file does not store, the runner removes the directory again
    /// before it reports the failure.
    pub fn acquire<C: ConsoleOps>(
        console: &mut C,
        host: &str,
        holder: &str,
        invocation: &str,
        transcript: &mut Transcript,
    ) -> Result<Self, LeaseError> {
        match console.make_dir(LEASE_DIR, transcript) {
            Ok(()) => {}
            Err(TransportError::UnexpectedReply { code: EXISTS, .. }) => {
                let holder = match console.fetch(HOLDER_FILE, transcript)? {
                    Some(bytes) => String::from_utf8_lossy(&bytes).trim().to_string(),
                    None => format!(
                        "an unnamed holder: {HOLDER_FILE} is absent, so a runner is taking \
                         the lease or one stopped before it named itself"
                    ),
                };
                return Err(LeaseError::Held {
                    host: host.to_string(),
                    holder,
                    unlock_with: format!("{invocation} unlock --host {host}"),
                });
            }
            Err(other) => return Err(other.into()),
        }
        if let Err(stored) = console.store(HOLDER_FILE, holder.as_bytes(), transcript) {
            if let Err(removed) = console.remove_dir(LEASE_DIR, transcript) {
                transcript.decision(format!("after the failed store, {removed}"));
            }
            return Err(stored.into());
        }
        Ok(Self { _held: () })
    }

    /// Remove the holder file and the lease directory.
    ///
    /// # Errors
    ///
    /// [`LeaseError::Transport`] when the console does not remove them;
    /// `unlock` clears what remains.
    pub fn release<C: ConsoleOps>(
        self,
        console: &mut C,
        transcript: &mut Transcript,
    ) -> Result<(), LeaseError> {
        console.delete(HOLDER_FILE, transcript)?;
        console.remove_dir(LEASE_DIR, transcript)?;
        Ok(())
    }
}

/// Remove the lease on `console` whoever holds it: every file in the
/// lease directory, then the directory. Returns whether a lease was
/// there.
///
/// # Errors
///
/// [`LeaseError::Transport`] when the console does not list or remove
/// it.
pub fn unlock<C: ConsoleOps>(
    console: &mut C,
    transcript: &mut Transcript,
) -> Result<bool, LeaseError> {
    let held = console
        .list(RESULT_ROOT, transcript)?
        .iter()
        .any(|name| name == LEASE_NAME);
    if !held {
        return Ok(false);
    }
    for name in console.list(LEASE_DIR, transcript)? {
        if name != "." && name != ".." {
            console.delete(&format!("{LEASE_DIR}/{name}"), transcript)?;
        }
    }
    console.remove_dir(LEASE_DIR, transcript)?;
    Ok(true)
}

#[cfg(test)]
#[path = "tests/lease_tests.rs"]
mod tests;
