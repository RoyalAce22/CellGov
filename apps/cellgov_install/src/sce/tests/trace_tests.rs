use super::*;

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

#[test]
fn an_unset_variable_leaves_the_section_trace_off() {
    assert!(!trace_enabled_for(None));
}

#[test]
fn a_set_but_off_value_leaves_the_section_trace_off() {
    for value in ["", "0", "false", "no", "off", "OFF", " off ", "\tFalse\n"] {
        assert!(!trace_enabled_for(Some(OsStr::new(value))), "{value:?}");
    }
}

#[test]
fn any_other_value_turns_the_section_trace_on() {
    for value in ["1", "true", "yes", "on", "ON", " 1 ", "sections"] {
        assert!(trace_enabled_for(Some(OsStr::new(value))), "{value:?}");
    }
}

#[test]
#[cfg(any(unix, windows))]
fn a_value_that_is_not_utf8_is_not_an_off_token() {
    let held = not_utf8();
    assert!(trace_enabled_for(Some(held.as_os_str())));
}

#[cfg(windows)]
fn not_utf8() -> std::ffi::OsString {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    // 0xD800 is an unpaired surrogate: UTF-16 holds it, UTF-8 cannot
    // encode it.
    OsString::from_wide(&[0xD800])
}

#[cfg(unix)]
fn not_utf8() -> std::ffi::OsString {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    OsString::from_vec(vec![0xFF])
}

/// Every `.rs` file under `dir`, relative to it, in `/`-separated form.
fn rust_sources(dir: &Path, prefix: &str, out: &mut Vec<(String, PathBuf)>) {
    for entry in std::fs::read_dir(dir).expect("source directory") {
        let path = entry.expect("source directory entry").path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .expect("source file name")
            .to_string();
        let rel = if prefix.is_empty() {
            name.clone()
        } else {
            format!("{prefix}/{name}")
        };
        if path.is_dir() {
            rust_sources(&path, &rel, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push((rel, path));
        }
    }
}

/// A second spelling of the variable lets the decrypt and the caller
/// disagree on the flag.
#[test]
fn only_the_trace_module_names_the_section_trace_variable() {
    let mut sources = Vec::new();
    rust_sources(
        &PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src"),
        "",
        &mut sources,
    );
    assert!(
        sources.len() > 10,
        "the scan found {} files; it is not reading the crate",
        sources.len()
    );
    let naming: BTreeSet<String> = sources
        .into_iter()
        .filter(|(_, path)| {
            std::fs::read_to_string(path)
                .expect("source file")
                .contains(ENV_FW_DEBUG)
        })
        .map(|(rel, _)| rel)
        .collect();
    assert_eq!(
        naming,
        BTreeSet::from(["sce/trace.rs".to_string()]),
        "{ENV_FW_DEBUG} must be spelled once, and read through section_trace_enabled"
    );
}
