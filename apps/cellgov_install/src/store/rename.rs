//! One retry policy for the store's publish and teardown renames.
//!
//! Win32 refuses a directory rename with `ERROR_ACCESS_DENIED` while
//! any handle is open under the source, and an on-access scanner opens
//! the tree the installer just wrote. The handle clears on its own
//! within seconds, so the policy retries a refusal of that kind over a
//! short bounded backoff before it reports it. One that outlives the
//! backoff is a real permission problem, and the policy reports it with
//! its attempt count.
//!
//! The transient class is Win32's alone. A POSIX rename is unaffected
//! by open descriptors, so on other hosts no refusal clears on its own
//! and the policy reports the first one.

use std::io;
use std::path::Path;
use std::time::Duration;

/// Attempts a rename gets, counting the first.
pub(crate) const RENAME_ATTEMPTS: u32 = 10;

/// The wait before the second attempt. Each later wait doubles it up
/// to [`MAX_BACKOFF`], so the whole backoff spans about eleven seconds.
const FIRST_BACKOFF: Duration = Duration::from_millis(100);

/// The longest single wait.
const MAX_BACKOFF: Duration = Duration::from_secs(2);

/// Win32 `ERROR_SHARING_VIOLATION`: the file-rename form of the
/// open-handle refusal, which `std` leaves uncategorized.
#[cfg(windows)]
const ERROR_SHARING_VIOLATION: i32 = 32;

/// A rename refused on every attempt it got.
#[derive(Debug, thiserror::Error)]
#[error("{source}{}", render_attempts(*attempts))]
pub struct RenameRefused {
    /// Attempts made, counting the first. One when the refusal was not
    /// of the transient kind.
    pub attempts: u32,
    /// The last refusal.
    #[source]
    pub source: io::Error,
}

fn render_attempts(attempts: u32) -> String {
    if attempts > 1 {
        format!(" (still refused after {attempts} attempts)")
    } else {
        String::new()
    }
}

/// Whether a refusal is the open-handle kind that clears on its own.
///
/// `ERROR_ACCESS_DENIED` is the directory form, which `std` reports as
/// `PermissionDenied`; `ERROR_SHARING_VIOLATION` is the file form,
/// which it reports under no kind at all.
#[cfg(windows)]
fn is_transient(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied || e.raw_os_error() == Some(ERROR_SHARING_VIOLATION)
}

/// Whether a refusal is the open-handle kind that clears on its own.
///
/// Never, off Windows. A POSIX rename proceeds over open descriptors.
/// `PermissionDenied` there is `EACCES` or `EPERM`, the parent
/// directory's mode or its sticky bit, and nothing clears either on its
/// own. `EBUSY` names a mount point or another process's root, which
/// stays in use.
#[cfg(not(windows))]
fn is_transient(_: &io::Error) -> bool {
    false
}

/// Rename `from` onto `to` and outwait a transient refusal.
///
/// Returns how many refusals it outwaited: zero when the first attempt
/// landed. The waits are host sleeps; they order nothing.
///
/// # Errors
///
/// [`RenameRefused`] when the host refused the last attempt too. It
/// carries that refusal and the attempt count.
pub fn rename_with_retry(from: &Path, to: &Path) -> Result<u32, RenameRefused> {
    retry(
        || std::fs::rename(from, to),
        std::thread::sleep,
        is_transient,
    )
}

/// The policy over an injected operation, wait, and transient class.
/// A test drives it with neither a filesystem nor a clock, so it holds
/// on every host.
fn retry(
    mut attempt: impl FnMut() -> io::Result<()>,
    mut wait: impl FnMut(Duration),
    transient: impl Fn(&io::Error) -> bool,
) -> Result<u32, RenameRefused> {
    let mut backoff = FIRST_BACKOFF;
    let mut refusals = 0u32;
    loop {
        match attempt() {
            Ok(()) => return Ok(refusals),
            Err(source) => {
                refusals += 1;
                if refusals >= RENAME_ATTEMPTS || !transient(&source) {
                    return Err(RenameRefused {
                        attempts: refusals,
                        source,
                    });
                }
                wait(backoff);
                backoff = (backoff * 2).min(MAX_BACKOFF);
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/rename_tests.rs"]
mod tests;
