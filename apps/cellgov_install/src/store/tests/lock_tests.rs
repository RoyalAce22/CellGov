//! What one claim excludes, what it leaves free, and what a terminated
//! holder leaves behind.

use std::io::{BufRead, BufReader};
use std::path::Path;
use std::process::{Command, Stdio};

use super::*;
use crate::scratch_dir::scratch;
use crate::store::layout::{staging_sibling, tombstone_sibling, TitleId, VersionKey};

/// Names the store root the spawned holder claims under, and selects
/// the holder. Without the variable the holder returns at once, so an
/// `--ignored` sweep cannot hang on it.
const HOLD_ROOT_ENV: &str = "CELLGOV_LOCK_HOLD_ROOT";

/// The holder prints this once it holds the claim.
const HELD_MARKER: &str = "cellgov-lock-held";

/// Test path of the holder, as libtest filters it.
const HOLDER_TEST: &str = "store::lock::tests::hold_lock_until_killed";

/// How long the holder waits to be killed, so a stray invocation ends
/// on its own.
const HOLD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

/// How many times a reader retries a killed holder's claim before it
/// calls the claim leaked.
const RELEASE_POLLS: u32 = 100;

/// How long a reader waits between those attempts.
const RELEASE_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(50);

fn base(title_id: &str) -> Artifact {
    Artifact::TitleBase {
        title_id: TitleId::new(title_id).expect("synthetic title id"),
    }
}

fn update(title_id: &str, version: &str) -> Artifact {
    Artifact::TitleUpdate {
        title_id: TitleId::new(title_id).expect("synthetic title id"),
        version: VersionKey::new(version).expect("synthetic version"),
    }
}

fn firmware(version: &str) -> Artifact {
    Artifact::Firmware {
        version: VersionKey::new(version).expect("synthetic version"),
    }
}

fn held_name(err: &StoreLockError) -> String {
    match err {
        StoreLockError::Contended { held, .. } => held.clone(),
        other => panic!("expected a contended claim, got {other}"),
    }
}

#[test]
fn a_second_claim_on_one_artifact_names_what_holds_it() {
    let out = scratch();
    let layout = StoreLayout::new(out.join("vfs"));
    let artifact = base("TEST00000");

    let _first = lock_artifact(&layout, &artifact).expect("the first claim");
    let err = lock_artifact(&layout, &artifact).expect_err("the second claim");

    assert_eq!(held_name(&err), "the base install of TEST00000");
    assert!(
        err.to_string()
            .contains(&layout.lock_path(&artifact).display().to_string()),
        "the refusal names the lock file: {err}"
    );
}

#[test]
fn dropping_the_guard_frees_the_artifact() {
    let out = scratch();
    let layout = StoreLayout::new(out.join("vfs"));
    let artifact = firmware("4.91");

    let first = lock_artifact(&layout, &artifact).expect("the first claim");
    // `is_err` alone would also pass on an unwritable lock directory.
    let err = lock_artifact(&layout, &artifact).expect_err("the second claim");
    assert_eq!(held_name(&err), "firmware 4.91");
    drop(first);

    let _second = lock_artifact(&layout, &artifact).expect("the artifact is free again");
}

#[test]
fn four_titles_are_claimed_independently() {
    let out = scratch();
    let layout = StoreLayout::new(out.join("vfs"));

    let held: Vec<StoreLock> = ["TEST00000", "TEST00001", "TEST00002", "TEST00003"]
        .iter()
        .map(|id| lock_artifact(&layout, &base(id)).expect("one title does not hold another"))
        .collect();

    assert_eq!(held.len(), 4);
}

#[test]
fn a_base_an_update_and_another_version_are_three_claims() {
    let out = scratch();
    let layout = StoreLayout::new(out.join("vfs"));

    let _b = lock_artifact(&layout, &base("TEST00000")).expect("the base");
    let _u = lock_artifact(&layout, &update("TEST00000", "02.10")).expect("one update");
    let _u2 = lock_artifact(&layout, &update("TEST00000", "02.51")).expect("another update");
}

#[test]
fn the_firmware_staging_claim_is_no_version_claim() {
    let out = scratch();
    let layout = StoreLayout::new(out.join("vfs"));

    let _staging = lock_firmware_staging(&layout).expect("the staging directory");
    let _version =
        lock_artifact(&layout, &firmware("4.91")).expect("a version is claimed separately");

    // `staging` passes `VersionKey`, so a version could otherwise reach
    // the staging directory's own lock file.
    assert_ne!(
        layout.lock_path(&firmware("staging")),
        layout.firmware_staging_lock_path()
    );
}

/// Every artifact kind, so no kind's lock stands in for another's.
fn every_kind() -> Vec<Artifact> {
    vec![
        firmware("4.91"),
        base("TEST00000"),
        update("TEST00000", "02.51"),
    ]
}

#[test]
fn a_lock_file_is_outside_every_directory_it_guards() {
    let layout = StoreLayout::new(Path::new("vfs"));

    for artifact in every_kind() {
        let lock = layout.lock_path(&artifact);
        let entry = layout.entry_dir(&artifact);
        let guarded = [
            entry.clone(),
            staging_sibling(&entry).expect("an entry names a staging sibling"),
            tombstone_sibling(&entry).expect("an entry names a tombstone sibling"),
            layout.title_dir(&TitleId::new("TEST00000").expect("synthetic title id")),
            layout.firmware_staging_dir(),
            layout.installs_dir(),
        ];
        for dir in guarded {
            assert!(
                !lock.starts_with(&dir),
                "{} guards {} and must not sit inside it",
                lock.display(),
                dir.display()
            );
            assert!(
                !dir.starts_with(layout.locks_dir()),
                "{} must not sit inside the locks directory",
                dir.display()
            );
        }
    }

    let staging_lock = layout.firmware_staging_lock_path();
    assert!(!staging_lock.starts_with(layout.firmware_staging_dir()));
    assert!(!staging_lock.starts_with(layout.firmware_root()));
}

/// A shared lock path would let one artifact's claim stand in for
/// another's.
#[test]
fn no_two_store_artifacts_share_one_lock_path() {
    let layout = StoreLayout::new(Path::new("vfs"));

    let mut artifacts = every_kind();
    artifacts.extend([
        firmware("4.90"),
        // `staging` and `staging.lock` pass `VersionKey`, so they reach
        // the firmware locks directory the staging claim also names.
        firmware("staging"),
        firmware("staging.lock"),
        base("TEST00001"),
        update("TEST00000", "02.10"),
        update("TEST00001", "02.51"),
    ]);

    let mut paths: Vec<(String, std::path::PathBuf)> = artifacts
        .iter()
        .map(|a| (format!("{a:?}"), layout.lock_path(a)))
        .collect();
    paths.push((
        "the firmware staging directory".to_string(),
        layout.firmware_staging_lock_path(),
    ));

    let mut compared = 0usize;
    for (i, (left_name, left)) in paths.iter().enumerate() {
        for (right_name, right) in &paths[i + 1..] {
            assert_ne!(
                left, right,
                "{left_name} and {right_name} resolve to one lock file"
            );
            compared += 1;
        }
    }
    // A shrunken list would leave the loop with nothing to assert.
    assert_eq!(compared, paths.len() * (paths.len() - 1) / 2);
    assert!(paths.len() >= 10, "{} lock paths compared", paths.len());
}

/// The staging lock's name is off the version keyspace. That keeps
/// [`no_two_store_artifacts_share_one_lock_path`] true for every
/// version, not only the versions it lists.
#[test]
fn no_version_key_starts_with_a_dot() {
    assert!(VersionKey::new(".staging").is_err());
    assert!(VersionKey::new(".staging.lock").is_err());
    assert!(TitleId::new(".staging").is_err());
}

/// A holder killed mid-install must leave the artifact free, since no
/// pass sweeps a lock file.
#[test]
fn a_terminated_holder_leaves_the_artifact_free() {
    let out = scratch();
    let vfs = out.join("vfs");
    let layout = StoreLayout::new(&vfs);
    let artifact = base("TEST00000");

    let exe = std::env::current_exe().expect("the test binary");
    let mut child = Command::new(exe)
        .args([HOLDER_TEST, "--exact", "--ignored", "--nocapture"])
        .env(HOLD_ROOT_ENV, &vfs)
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn the holder");

    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut line = String::new();
    loop {
        line.clear();
        let read = stdout
            .read_line(&mut line)
            .expect("read the holder's stdout");
        assert_ne!(read, 0, "the holder exited before it took the claim");
        if line.contains(HELD_MARKER) {
            break;
        }
    }

    let err = lock_artifact(&layout, &artifact).expect_err("the holder is alive");
    assert_eq!(held_name(&err), "the base install of TEST00000");

    child.kill().expect("kill the holder");
    child.wait().expect("reap the holder");

    // Win32 releases the file locks of a killed process on its own
    // schedule, so the claim can still read as held after the reap. The
    // bound makes a genuine leak fail rather than hang.
    let mut last = None;
    for _ in 0..RELEASE_POLLS {
        match lock_artifact(&layout, &artifact) {
            Ok(_) => return,
            Err(e) => last = Some(e),
        }
        std::thread::sleep(RELEASE_POLL_INTERVAL);
    }
    panic!(
        "a killed holder still holds its claim after {:?}: {}",
        RELEASE_POLLS * RELEASE_POLL_INTERVAL,
        last.expect("a refusal from every attempt")
    );
}

/// Claims a title base under `CELLGOV_LOCK_HOLD_ROOT` and waits to be
/// killed. Driven by [`a_terminated_holder_leaves_the_artifact_free`].
#[test]
#[ignore = "a fixture process, not a test: without CELLGOV_LOCK_HOLD_ROOT it asserts nothing"]
fn hold_lock_until_killed() {
    let Ok(root) = std::env::var(HOLD_ROOT_ENV) else {
        return;
    };
    let layout = StoreLayout::new(root);
    let _held = lock_artifact(&layout, &base("TEST00000")).expect("claim the base");
    println!("{HELD_MARKER}");
    std::thread::sleep(HOLD_TIMEOUT);
}
