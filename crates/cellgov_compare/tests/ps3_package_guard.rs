//! Convention guard: the console package keeps every `rldicr` the
//! compiler emitted.
//!
//! `tests/micro/common/patch_toc.py` rewrites `rldicr rN,rN,16,47`
//! (`sldi rN,rN,16`) into a 32-bit shift for the reference ELF. The
//! rewrite clears the high word, so in a console build it changes the
//! value of a 64-bit constant the probe sends. `package_ps3.sh` passes
//! `--toc-only` to keep it off the console ELF.

use std::fs;
use std::path::{Path, PathBuf};

/// The flag that stops the rewrite.
const TOC_ONLY: &str = "--toc-only";

fn common_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/micro/common")
}

fn read(name: &str) -> String {
    let path = common_dir().join(name);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Each `patch_toc.py` invocation in `script`, with its `\`
/// continuation lines joined.
fn patch_toc_calls(script: &str) -> Vec<String> {
    let lines: Vec<&str> = script.lines().collect();
    let mut calls = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        if line.trim_start().starts_with('#') || !line.contains("patch_toc.py") {
            continue;
        }
        let mut call = String::new();
        for next in &lines[i..] {
            call.push_str(next.trim_end_matches('\\').trim());
            call.push(' ');
            if !next.trim_end().ends_with('\\') {
                break;
            }
        }
        calls.push(call);
    }
    calls
}

#[test]
fn the_console_package_runs_patch_toc_with_toc_only() {
    let calls = patch_toc_calls(&read("package_ps3.sh"));
    assert!(!calls.is_empty(), "package_ps3.sh runs patch_toc.py");
    for call in &calls {
        assert!(
            call.split_whitespace().any(|arg| arg == TOC_ONLY),
            "package_ps3.sh runs patch_toc.py without {TOC_ONLY}: {call}"
        );
    }
}

#[test]
fn patch_toc_accepts_toc_only() {
    let script = read("patch_toc.py");
    assert!(
        script.contains(&format!("\"{TOC_ONLY}\"")),
        "patch_toc.py does not handle {TOC_ONLY}"
    );
}

#[test]
fn a_continued_call_is_read_to_its_last_line() {
    let calls = patch_toc_calls(
        "x\n    python3 \"$C/patch_toc.py\" \\\n        a \\\n        --toc-only\ny\n",
    );
    assert_eq!(
        calls,
        vec!["python3 \"$C/patch_toc.py\" a --toc-only ".to_string()]
    );
}
