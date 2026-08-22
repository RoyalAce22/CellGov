//! The operator's dump root: the one corpus location a run may
//! relocate.
//!
//! Dumps are the operator's own PKG / disc / PUP files, often too
//! large to sit inside the checkout, so `CELLGOV_DUMPS_DIR` may point
//! elsewhere. The override selects where fixtures are read from --
//! never whether a test runs or what it asserts.
//!
//! Expected layout:
//!
//! ```text
//! <dumps>/firmware/PS3UPDAT.PUP
//! <dumps>/<content-id>/<content-id>.pkg   (+ .rap)
//! <dumps>/<content-id>/<content-id>.iso
//! ```

#![allow(
    dead_code,
    reason = "each integration-test binary compiles this module separately and uses a subset"
)]

use std::ffi::OsString;
use std::path::PathBuf;

/// Env var naming an alternate dump root.
pub const ENV_DUMPS_DIR: &str = "CELLGOV_DUMPS_DIR";

/// Workspace root, resolved from this crate's manifest dir.
pub fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// Root holding per-title dump directories and `firmware/`.
///
/// # Panics
///
/// If [`ENV_DUMPS_DIR`] is set to an empty value.
pub fn root() -> PathBuf {
    root_from(std::env::var_os(ENV_DUMPS_DIR))
}

/// [`root`] with the override handed in.
///
/// # Panics
///
/// If the override is set but empty; an empty value resolves against
/// the process working directory.
fn root_from(overridden: Option<OsString>) -> PathBuf {
    match overridden {
        // A dump root outside the checkout can be a non-UTF-8 path,
        // which `var` reports as an absent override.
        Some(v) if !v.is_empty() => PathBuf::from(v),
        Some(_) => panic!(
            "{ENV_DUMPS_DIR} is set but empty; point it at a dump root \
             or unset it to use the in-checkout default"
        ),
        None => workspace_root().join("dumps"),
    }
}

/// Dump directory for one title.
pub fn title_dir(content_id: &str) -> PathBuf {
    root().join(content_id)
}

/// The console update package a firmware install reads.
pub fn ps3updat_pup() -> PathBuf {
    root().join("firmware").join("PS3UPDAT.PUP")
}

#[test]
fn an_unset_dump_root_falls_back_to_the_in_checkout_default() {
    assert_eq!(root_from(None), workspace_root().join("dumps"));
}

#[test]
fn a_dump_root_override_is_taken_verbatim() {
    let over = OsString::from("D:/elsewhere/ps3dumps");
    assert_eq!(
        root_from(Some(over)),
        PathBuf::from("D:/elsewhere/ps3dumps")
    );
}

#[test]
fn an_empty_dump_root_override_is_named_rather_than_resolved_against_the_cwd() {
    let payload = std::panic::catch_unwind(|| root_from(Some(OsString::new())))
        .err()
        .unwrap_or_else(|| panic!("an empty {ENV_DUMPS_DIR} was accepted"));
    let msg = payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| panic!("the refusal carried a non-string payload"));
    assert!(msg.contains("set but empty"), "got {msg:?}");
}
