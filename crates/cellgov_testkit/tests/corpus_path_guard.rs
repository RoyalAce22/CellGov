//! Convention guard: no source reads an RPCS3 install tree.
//!
//! CellGov owns its corpus. Firmware comes from `cellgov_install
//! install` into `vfs/dev_flash/`, titles from `install-game` /
//! `install-iso` into `vfs/dev_hdd0/` and `vfs/dev_bdvd/`, and
//! anything RPCS3 alone can answer is captured into committed data
//! under `tests/fixtures/`.
//!
//! The RPCS3 *source* checkout (`tools/rpcs3-src/`) is a different
//! directory and stays allowed.

use std::fs;
use std::path::{Path, PathBuf};

/// Assembled at run time so this file does not itself contain the
/// banned needle and can be scanned like every other source.
fn banned_prefix() -> String {
    ["tools", "rpcs3"].join("/")
}

/// Suffix that makes the RPCS3 source checkout a directory of its own
/// beside the install tree.
const SOURCE_CHECKOUT_SUFFIX: &str = "-src";

/// Fold a line to one spelling: ASCII-lowercase, backslash separators
/// to forward slashes, runs of separators to one, and `/./` to `/`.
///
/// Without it a Windows-style, uppercase, doubled-separator, or
/// dot-component spelling of the same tree slips past a plain
/// substring test.
fn normalize(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for ch in line.chars() {
        let ch = if ch == '\\' {
            '/'
        } else {
            ch.to_ascii_lowercase()
        };
        if ch == '/' {
            if out.ends_with('/') {
                continue;
            }
            if out.ends_with("/.") {
                out.pop();
                continue;
            }
        }
        out.push(ch);
    }
    out
}

/// Whether `line` names the install tree, as a whole path component.
///
/// A trailing separator is not required -- a bare prefix reaches the
/// same tree through one `join` -- but the character after the prefix
/// must not be a name character, or every sibling directory that
/// merely starts with the same letters gets flagged too. The sibling
/// that matters is [`SOURCE_CHECKOUT_SUFFIX`].
fn names_install_tree(line: &str, needle: &str) -> bool {
    let normalized = normalize(line);
    normalized.match_indices(needle).any(|(at, _)| {
        normalized[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    })
}

fn rs_files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("cannot read entry under {}: {e}", dir.display()));
        let kind = entry
            .file_type()
            .unwrap_or_else(|e| panic!("cannot stat {}: {e}", entry.path().display()));
        if kind.is_symlink() {
            continue;
        }
        let path = entry.path();
        if kind.is_dir() {
            if path
                .file_name()
                .is_some_and(|n| n == "target" || n.to_string_lossy().starts_with('.'))
            {
                continue;
            }
            rs_files_under(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

/// Spellings are assembled, never written literally: a literal would
/// make this file a violation of the guard it defines.
#[test]
fn matcher_catches_every_spelling_of_the_install_tree() {
    let needle = banned_prefix();
    let install = ["tools", "rpcs3"].join("/");
    for spelling in [
        install.clone(),
        install.clone() + "/dev_hdd0/game",
        install.to_uppercase(),
        ["tools", "rpcs3", ""].join("\\"),
        ["tools", "", "rpcs3"].join("/"),
        ["tools", ".", "rpcs3"].join("/"),
    ] {
        assert!(
            names_install_tree(&format!("    let p = \"../../{spelling}\";"), &needle),
            "spelling {spelling:?} slipped past the matcher"
        );
    }
}

#[test]
fn matcher_leaves_the_source_checkout_alone() {
    let needle = banned_prefix();
    let checkout = ["tools", "rpcs3"].join("/") + SOURCE_CHECKOUT_SUFFIX;
    for spelling in [
        checkout.clone(),
        checkout.clone() + "/rpcs3/Emu/Cell",
        checkout.to_uppercase(),
        ["tools", "rpcs3-src", "build-msvc"].join("\\"),
        ["tools", "", "rpcs3-src"].join("/"),
    ] {
        assert!(
            !names_install_tree(&format!("//! see {spelling}"), &needle),
            "the source checkout {spelling:?} was flagged as an install tree"
        );
    }
}

#[test]
fn matcher_leaves_siblings_that_merely_share_the_prefix_alone() {
    let needle = banned_prefix();
    let base = ["tools", "rpcs3"].join("/");
    for tail in ["src", "-source", "_src", "-patch", "-"] {
        let spelling = base.clone() + tail;
        assert!(
            !names_install_tree(&format!("//! see {spelling}"), &needle),
            "{spelling:?} is not the install tree but was flagged"
        );
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

/// Every `.rs` file the guard is responsible for.
fn scanned_rs_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for group in ["crates", "apps", "bridges"] {
        rs_files_under(&root.join(group), &mut files);
    }
    files
}

/// The scan reaches nested `tests/` directories, so an empty result is
/// a broken walk rather than a clean workspace.
#[test]
fn the_scan_set_contains_this_guard() {
    let root = workspace_root();
    let files = scanned_rs_files(&root);
    let me = root
        .join("crates")
        .join("cellgov_testkit")
        .join("tests")
        .join("corpus_path_guard.rs");
    assert!(
        files.contains(&me),
        "the scan did not reach {}; {} files were collected",
        me.display(),
        files.len()
    );
}

#[test]
fn no_source_reads_an_rpcs3_install_tree() {
    let needle = banned_prefix();

    let files = scanned_rs_files(&workspace_root());
    assert!(
        !files.is_empty(),
        "no .rs files found under crates/apps/bridges"
    );

    let mut violations = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        for (n, line) in source.lines().enumerate() {
            if names_install_tree(line, &needle) {
                violations.push((file.clone(), n + 1));
            }
        }
    }
    violations.sort();

    let mut report = String::new();
    for (file, line) in &violations {
        report.push_str(&format!("  {}:{line}\n", file.display()));
    }
    assert!(
        violations.is_empty(),
        "sources naming an RPCS3 install tree. CellGov's corpus is \
         vfs/ (from cellgov_install) plus committed data under \
         tests/fixtures/; an RPCS3 install is one operator's machine \
         state, not a fixture location. The RPCS3 source checkout \
         (tools/rpcs3-src/) is allowed and unaffected:\n{report}"
    );
}
