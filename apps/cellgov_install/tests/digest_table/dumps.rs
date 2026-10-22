//! The dump-root resolution rules.

use std::ffi::OsString;
use std::path::PathBuf;

#[path = "../common/dumps.rs"]
mod root;
pub use root::*;

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
