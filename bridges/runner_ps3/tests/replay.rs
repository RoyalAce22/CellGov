//! Every committed console capture reproduces: for each
//! `tests/micro/<name>/ps3/<profile>/`, the capture loads with every
//! check the reference lookup makes, and `convert` on its frame
//! reproduces its `observation.json` byte for byte.
//!
//! The changes this must refuse (a changed frame byte, a capture moved
//! under another profile, an observation its frame does not produce)
//! are measured on synthetic captures in the crate's capture tests, so
//! this target stays a real check while few or no captures are
//! committed.

use std::fs;
use std::path::{Path, PathBuf};

use cellgov_compare::console_profile::{ConsoleProfiles, CONSOLE_PROFILES_FILE};
use cellgov_compare::hardware_capture::CAPTURE_DIR;
use runner_ps3::capture::replay;

fn micro_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/micro")
}

fn subdirectories(dir: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|entry| entry.expect("directory entry").path())
        .filter(|path| path.is_dir())
        .collect();
    dirs.sort();
    dirs
}

/// Every `ps3/<profile>/` directory under a microtest, with its manifest.
fn committed_captures() -> Vec<(PathBuf, PathBuf)> {
    let mut captures = Vec::new();
    for test in subdirectories(&micro_root()) {
        let root = test.join(CAPTURE_DIR);
        if root.is_dir() {
            for capture in subdirectories(&root) {
                captures.push((capture, test.join("manifest.toml")));
            }
        }
    }
    captures
}

#[test]
fn every_committed_capture_replays_from_its_frame() {
    let profiles = ConsoleProfiles::load(&micro_root().join(CONSOLE_PROFILES_FILE))
        .expect("the tracked profiles load");
    let captures = committed_captures();
    let mut problems = Vec::new();
    for (capture, manifest) in &captures {
        if let Err(error) = replay(capture, manifest, &profiles) {
            problems.push(format!("  {}: {error}", capture.display()));
        }
    }
    assert!(
        problems.is_empty(),
        "{} of {} committed captures do not replay:\n{}",
        problems.len(),
        captures.len(),
        problems.join("\n")
    );
}
