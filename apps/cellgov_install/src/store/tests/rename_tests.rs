//! The rename policy: which refusals it outwaits, for how long, and
//! what it reports either way.

use std::io;
use std::time::Duration;

use super::*;
use crate::scratch_dir::scratch;

fn denied() -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, "Access is denied.")
}

/// The transient class the policy tests inject, so they hold on every
/// host. The per-platform tests below hold the host's own class.
fn denied_is_transient(e: &io::Error) -> bool {
    e.kind() == io::ErrorKind::PermissionDenied
}

#[test]
fn a_rename_that_lands_first_time_waits_for_nothing() {
    let mut waits = Vec::new();
    let retries = retry(|| Ok(()), |d| waits.push(d), denied_is_transient).expect("landed");
    assert_eq!(retries, 0);
    assert!(waits.is_empty());
}

#[test]
fn a_transient_refusal_is_outwaited_with_a_doubling_backoff() {
    let mut refusals_left = 2;
    let mut waits = Vec::new();
    let retries = retry(
        || {
            if refusals_left > 0 {
                refusals_left -= 1;
                Err(denied())
            } else {
                Ok(())
            }
        },
        |d| waits.push(d),
        denied_is_transient,
    )
    .expect("landed on the third attempt");
    assert_eq!(retries, 2);
    assert_eq!(
        waits,
        [Duration::from_millis(100), Duration::from_millis(200)]
    );
}

#[test]
fn a_refusal_that_outlives_the_backoff_reports_every_attempt() {
    let mut attempts = 0;
    let mut waits = Vec::new();
    let err = retry(
        || {
            attempts += 1;
            Err(denied())
        },
        |d| waits.push(d),
        denied_is_transient,
    )
    .expect_err("never lands");
    assert_eq!(err.attempts, RENAME_ATTEMPTS);
    assert_eq!(attempts, RENAME_ATTEMPTS);
    assert_eq!(
        u32::try_from(waits.len()).unwrap(),
        RENAME_ATTEMPTS - 1,
        "no wait follows the last attempt"
    );
    assert_eq!(err.source.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(
        err.to_string(),
        format!("Access is denied. (still refused after {RENAME_ATTEMPTS} attempts)")
    );
    assert!(
        waits.iter().all(|w| *w <= MAX_BACKOFF),
        "no single wait outgrows the cap: {waits:?}"
    );
    let total: Duration = waits.iter().sum();
    assert!(
        total <= Duration::from_secs(15),
        "the backoff spans seconds, not minutes: {total:?}"
    );
}

#[test]
fn a_refusal_of_another_kind_is_not_retried() {
    let mut attempts = 0;
    let mut waits = Vec::new();
    let err = retry(
        || {
            attempts += 1;
            Err(io::Error::new(io::ErrorKind::NotFound, "no such tree"))
        },
        |d| waits.push(d),
        denied_is_transient,
    )
    .expect_err("refused at once");
    assert_eq!(err.attempts, 1);
    assert_eq!(attempts, 1);
    assert!(waits.is_empty());
    assert_eq!(
        err.to_string(),
        "no such tree",
        "a single attempt carries no attempt count"
    );
}

#[test]
fn a_missing_source_is_refused_without_a_wait() {
    let dir = scratch();
    let err =
        rename_with_retry(&dir.join("absent"), &dir.join("dst")).expect_err("nothing to rename");
    assert_eq!(err.attempts, 1);
    assert_eq!(err.source.kind(), io::ErrorKind::NotFound);
}

#[test]
fn a_present_source_lands_first_time() {
    let dir = scratch();
    let src = dir.join("src");
    let dst = dir.join("dst");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("sub").join("a"), b"a").unwrap();

    assert_eq!(rename_with_retry(&src, &dst).expect("lands"), 0);
    assert!(dst.join("sub").join("a").exists());
    assert!(!src.exists());
}

#[cfg(windows)]
#[test]
fn on_windows_access_denied_and_sharing_violation_are_transient() {
    const ERROR_FILE_NOT_FOUND: i32 = 2;
    const ERROR_ACCESS_DENIED: i32 = 5;

    let denied = io::Error::from_raw_os_error(ERROR_ACCESS_DENIED);
    assert_eq!(denied.kind(), io::ErrorKind::PermissionDenied);
    assert!(is_transient(&denied));

    let sharing = io::Error::from_raw_os_error(ERROR_SHARING_VIOLATION);
    assert_ne!(
        sharing.kind(),
        io::ErrorKind::PermissionDenied,
        "the file form is caught by its raw code, not its kind"
    );
    assert!(is_transient(&sharing));

    assert!(!is_transient(&io::Error::from_raw_os_error(
        ERROR_FILE_NOT_FOUND
    )));
}

#[cfg(not(windows))]
#[test]
fn off_windows_no_refusal_is_transient() {
    assert!(!is_transient(&denied()));
    assert!(!is_transient(&io::Error::new(
        io::ErrorKind::ResourceBusy,
        "in use"
    )));
    assert!(!is_transient(&io::Error::new(
        io::ErrorKind::NotFound,
        "absent"
    )));
}

/// The test holds the handle across the first attempt and releases it
/// in the first wait. The refusal therefore does not depend on how soon
/// this thread reaches the rename. The wait still sleeps, so the test
/// outwaits a scanner's own handle on the scratch tree as the policy
/// would in service.
#[cfg(windows)]
#[test]
fn an_open_handle_under_the_source_is_outwaited() {
    let dir = scratch();
    let src = dir.join("src");
    let dst = dir.join("dst");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("sub").join("a"), b"a").unwrap();

    let mut held = Some(std::fs::File::open(src.join("sub").join("a")).unwrap());
    let mut first_refusal = None;
    let retries = retry(
        || {
            let outcome = std::fs::rename(&src, &dst);
            if let Err(e) = &outcome {
                if first_refusal.is_none() {
                    first_refusal = Some(e.kind());
                }
            }
            outcome
        },
        |d| {
            drop(held.take());
            std::thread::sleep(d);
        },
        is_transient,
    )
    .expect("lands once the handle closes");
    assert!(
        retries >= 1,
        "the open handle refused at least the first attempt"
    );
    assert_eq!(
        first_refusal,
        Some(io::ErrorKind::PermissionDenied),
        "Win32 reports the open child as ERROR_ACCESS_DENIED"
    );
    assert!(dst.join("sub").join("a").exists());
    assert!(!src.exists());
}
