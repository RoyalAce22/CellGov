//! Convention guard: one source of temporary test paths.
//!
//! Two rules, both about `cellgov_testkit::scratch`:
//!
//! 1. No file under `crates/`, `apps/` or `bridges/` calls
//!    `std::env::temp_dir()`, except the sites [`ALLOWED`] names. A
//!    hand-rolled scratch path leaks whenever an assertion fails, and
//!    a leaked fixture tree costs gigabytes.
//! 2. Only a `[dev-dependencies]` entry turns the `scratch` feature
//!    on. The feature pulls `tempfile` in, and shipped binaries
//!    depend on `cellgov_testkit` at runtime.
//!
//! This guard names `std::env::temp_dir()` only inside comments and
//! string literals, both of which the scan masks.

use std::fs;
use std::path::{Path, PathBuf};

/// Files that may call `std::env::temp_dir()`, workspace-relative with
/// `/` separators, each with the reason it is not the helper's job.
const ALLOWED: [(&str, &str); 2] = [
    (
        "crates/cellgov_testkit/src/tests/scratch_tests.rs",
        "asserts where the helper puts a directory",
    ),
    (
        "apps/cellgov_cli/src/game/bench/divergence.rs",
        "a shipped diagnostic writes two state traces and removes them; \
         the helper is a dev-dependency and never reaches a release build",
    ),
];

fn rs_files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("cannot read entry under {}: {e}", dir.display()));
        let kind = entry
            .file_type()
            .unwrap_or_else(|e| panic!("cannot stat {}: {e}", entry.path().display()));
        // `file_type` does not follow links, so a symlinked directory
        // cannot walk the scan out of the workspace or into a cycle.
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

/// Line number (1-indexed) of byte offset `at` in `source`.
fn line_of(source: &str, at: usize) -> usize {
    source[..at].matches('\n').count() + 1
}

/// Length in bytes of the UTF-8 character starting with `first`.
fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        _ => 4,
    }
}

fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// `source` with comment bodies and string/char-literal contents
/// blanked to spaces.
///
/// Byte offsets and line numbers are preserved -- newlines survive and
/// every replacement is one ASCII space per byte -- so a match in the
/// result indexes straight back into `source`.
fn mask_comments_and_literals(source: &str) -> String {
    let b = source.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for byte in &mut out[from..to.min(b.len())] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };

    let mut i = 0;
    while i < b.len() {
        // Line comment.
        if b[i] == b'/' && b.get(i + 1) == Some(&b'/') {
            let end = source[i..].find('\n').map_or(b.len(), |off| i + off);
            blank(&mut out, i, end);
            i = end;
            continue;
        }
        // Block comment, which nests in Rust.
        if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
            let start = i;
            let mut depth = 1u32;
            i += 2;
            while i < b.len() && depth > 0 {
                if b[i] == b'/' && b.get(i + 1) == Some(&b'*') {
                    depth += 1;
                    i += 2;
                } else if b[i] == b'*' && b.get(i + 1) == Some(&b'/') {
                    depth -= 1;
                    i += 2;
                } else {
                    i += 1;
                }
            }
            blank(&mut out, start, i);
            continue;
        }
        // Raw string, with an optional `b` byte-string prefix. Only at
        // a token start, so the `r` of an identifier is not a prefix.
        if (b[i] == b'r' || (b[i] == b'b' && b.get(i + 1) == Some(&b'r')))
            && (i == 0 || !is_ident_byte(b[i - 1]))
        {
            let mut j = if b[i] == b'b' { i + 2 } else { i + 1 };
            let hash_start = j;
            while b.get(j) == Some(&b'#') {
                j += 1;
            }
            if b.get(j) == Some(&b'"') {
                let hashes = j - hash_start;
                let mut close = String::from('"');
                close.push_str(&"#".repeat(hashes));
                let body = j + 1;
                let end = source[body..]
                    .find(&close)
                    .map_or(b.len(), |off| body + off + close.len());
                blank(&mut out, body, end.saturating_sub(close.len()));
                i = end;
                continue;
            }
        }
        // Ordinary string, with an optional `b` byte-string prefix.
        if b[i] == b'"' {
            let body = i + 1;
            let mut j = body;
            while j < b.len() && b[j] != b'"' {
                j += if b[j] == b'\\' { 2 } else { 1 };
            }
            blank(&mut out, body, j.min(b.len()));
            i = (j + 1).min(b.len());
            continue;
        }
        // Char literal, distinguished from a lifetime or loop label by
        // the closing quote one character later.
        if b[i] == b'\'' {
            let escaped = b.get(i + 1) == Some(&b'\\');
            let closes_at = if escaped {
                // `'\n'`, `'\''`, `'\u{1f}'` -- scan to the next quote.
                let mut j = i + 2;
                while j < b.len() && b[j] != b'\'' {
                    j += 1;
                }
                (j < b.len()).then_some(j)
            } else {
                let after = i + 1 + b.get(i + 1).copied().map_or(0, utf8_len);
                (b.get(after) == Some(&b'\'')).then_some(after)
            };
            if let Some(close) = closes_at {
                blank(&mut out, i + 1, close);
                i = close + 1;
                continue;
            }
        }
        i += 1;
    }
    String::from_utf8(out).expect("blanking bytes with ASCII spaces preserves UTF-8")
}

/// Lines (1-indexed) of every `std::env::temp_dir()` call in `source`.
fn call_sites(source: &str) -> Vec<usize> {
    let masked = mask_comments_and_literals(source);
    masked
        .match_indices("temp_dir()")
        // `fn fresh_temp_dir()` and calls to it are not the std call.
        .filter(|(at, _)| *at == 0 || !is_ident_byte(masked.as_bytes()[*at - 1]))
        .map(|(at, _)| line_of(source, at))
        .collect()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

fn scanned_rs_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for group in ["crates", "apps", "bridges"] {
        rs_files_under(&root.join(group), &mut files);
    }
    files
}

/// `file` relative to `root`, `/`-separated so [`ALLOWED`] reads the
/// same on either host.
fn rel(root: &Path, file: &Path) -> String {
    file.strip_prefix(root)
        .unwrap_or(file)
        .to_string_lossy()
        .replace('\\', "/")
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
        .join("scratch_dir_guard.rs");
    assert!(
        files.contains(&me),
        "the scan did not reach {}; {} files were collected",
        me.display(),
        files.len()
    );
}

/// Floor on the population the rule polices.
///
/// The value sits well under the current count, so ordinary churn does
/// not trip it.
const MIN_HELPER_CALLS: usize = 150;

/// Helper calls in `source`, comments and string literals masked out.
fn helper_calls(source: &str) -> usize {
    let masked = mask_comments_and_literals(source);
    masked.matches("scratch_labeled(").count() + masked.matches("scratch()").count()
}

#[test]
fn the_helper_has_callers_across_the_workspace() {
    let root = workspace_root();
    let mut calls = 0usize;
    for file in scanned_rs_files(&root) {
        if rel(&root, &file).starts_with("crates/cellgov_testkit/") {
            continue;
        }
        let source = fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        calls += helper_calls(&source);
    }
    assert!(
        calls >= MIN_HELPER_CALLS,
        "gate went vacuous: only {calls} call(s) of the scratch helper \
         outside cellgov_testkit, expected at least {MIN_HELPER_CALLS}"
    );
}

/// The workspace-wide floor cannot see one crate that takes its
/// scratch paths back. The other crates' calls keep the total over any
/// threshold worth setting.
#[test]
fn every_crate_that_declares_the_feature_still_calls_the_helper() {
    let root = workspace_root();
    let files = scanned_rs_files(&root);
    let mut checked = 0usize;
    for manifest in scanned_manifests(&root) {
        let text = fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
        if !declares_scratch_feature(&text) {
            continue;
        }
        let Some(dir) = manifest.parent() else {
            continue;
        };
        let prefix = rel(&root, dir) + "/";
        let calls: usize = files
            .iter()
            .filter(|f| rel(&root, f).starts_with(&prefix))
            .map(|f| {
                let source = fs::read_to_string(f)
                    .unwrap_or_else(|e| panic!("cannot read {}: {e}", f.display()));
                helper_calls(&source)
            })
            .sum();
        assert!(
            calls > 0,
            "{prefix} declares the scratch feature but calls the helper \
             nowhere; either it took its scratch paths back, or the \
             dependency is dead and should go"
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "no manifest declares the scratch feature, so this gate inspected nothing"
    );
}

/// Whether `manifest` turns the helper's feature on anywhere.
fn declares_scratch_feature(manifest: &str) -> bool {
    manifest
        .lines()
        .map(|raw| raw.split('#').next().unwrap_or("").trim())
        .any(|line| line.starts_with("cellgov_testkit") && line.contains("scratch"))
}

#[test]
fn no_file_outside_the_helper_reaches_for_the_temp_directory() {
    let root = workspace_root();
    let files = scanned_rs_files(&root);
    assert!(
        !files.is_empty(),
        "no .rs files found under crates/apps/bridges"
    );

    let mut violations = Vec::new();
    for file in &files {
        let name = rel(&root, file);
        if ALLOWED.iter().any(|(allowed, _)| *allowed == name) {
            continue;
        }
        let source = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        for line in call_sites(&source) {
            violations.push(format!("  {name}:{line}\n"));
        }
    }
    violations.sort();

    let report: String = violations.concat();
    assert!(
        violations.is_empty(),
        "these reach for the OS temp directory directly, and a scratch \
         path that is not the helper's leaks whenever an assertion \
         fails. Use cellgov_testkit::scratch::scratch() or \
         scratch_labeled(label), and hold the guard for as long as the \
         path is read:\n{report}"
    );
}

/// A rename must not leave a permission behind that covers nothing.
#[test]
fn every_allowance_names_a_file_that_exists() {
    let root = workspace_root();
    for (name, reason) in ALLOWED {
        assert!(
            root.join(name).is_file(),
            "allowance {name:?} ({reason}) names no file"
        );
    }
}

#[test]
fn every_allowance_still_makes_the_call_it_permits() {
    let root = workspace_root();
    for (name, reason) in ALLOWED {
        let source = fs::read_to_string(root.join(name))
            .unwrap_or_else(|e| panic!("cannot read {name}: {e}"));
        assert!(
            !call_sites(&source).is_empty(),
            "allowance {name:?} ({reason}) no longer calls temp_dir(); drop the row"
        );
    }
}

/// Lines (1-indexed) of `manifest` that turn the `scratch` feature on
/// from a runtime dependency table.
///
/// This reads two spellings:
///
/// - a `cellgov_testkit = { .., features = [..] }` entry under
///   `[dependencies]`
/// - a `[dependencies.cellgov_testkit]` table of its own
fn runtime_scratch_enablers(manifest: &str) -> Vec<usize> {
    let mut hits = Vec::new();
    let mut runtime = false;
    let mut own_table = false;
    for (i, raw) in manifest.lines().enumerate() {
        let line = raw.split('#').next().unwrap_or("").trim();
        if let Some(header) = line.strip_prefix('[').and_then(|h| h.strip_suffix(']')) {
            runtime = header.contains("dependencies")
                && !header.contains("dev-dependencies")
                && !header.contains("build-dependencies");
            own_table = header.ends_with(".cellgov_testkit");
            continue;
        }
        if !runtime {
            continue;
        }
        let names_the_dep = own_table || line.starts_with("cellgov_testkit");
        if names_the_dep && line.contains("scratch") {
            hits.push(i + 1);
        }
    }
    hits
}

/// Manifests of the workspace members, one directory deep under each
/// scanned group.
fn scanned_manifests(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for group in ["crates", "apps", "bridges"] {
        let dir = root.join(group);
        let entries =
            fs::read_dir(&dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
        for entry in entries {
            let entry =
                entry.unwrap_or_else(|e| panic!("cannot read entry under {}: {e}", dir.display()));
            let manifest = entry.path().join("Cargo.toml");
            if manifest.is_file() {
                out.push(manifest);
            }
        }
    }
    out
}

/// Floor on the manifest population, so a broken walk is not a pass.
const MIN_MANIFESTS: usize = 15;

#[test]
fn no_runtime_dependency_turns_the_scratch_feature_on() {
    let root = workspace_root();
    let manifests = scanned_manifests(&root);
    assert!(
        manifests.len() >= MIN_MANIFESTS,
        "manifest scan went vacuous: {} found, expected at least {MIN_MANIFESTS}",
        manifests.len()
    );

    let mut hits = Vec::new();
    for manifest in &manifests {
        let text = fs::read_to_string(manifest)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", manifest.display()));
        for line in runtime_scratch_enablers(&text) {
            hits.push(format!("  {}:{line}\n", rel(&root, manifest)));
        }
    }
    hits.sort();

    let report: String = hits.concat();
    assert!(
        hits.is_empty(),
        "the scratch feature pulls `tempfile` in, and resolver 2 only \
         keeps it out of a normal build while every enabler sits under \
         [dev-dependencies]. Move these:\n{report}"
    );
}

#[test]
fn a_runtime_dependency_on_the_scratch_feature_is_an_enabler() {
    let manifest =
        "[dependencies]\ncellgov_testkit = { path = \"..\", features = [\"scratch\"] }\n";
    assert_eq!(runtime_scratch_enablers(manifest), vec![2]);
}

#[test]
fn a_runtime_dependency_in_a_table_of_its_own_is_an_enabler() {
    let manifest = "[dependencies.cellgov_testkit]\npath = \"..\"\nfeatures = [\"scratch\"]\n";
    assert_eq!(runtime_scratch_enablers(manifest), vec![3]);
}

#[test]
fn a_dev_dependency_on_the_scratch_feature_is_not_an_enabler() {
    let manifest =
        "[dev-dependencies]\ncellgov_testkit = { path = \"..\", features = [\"scratch\"] }\n";
    assert!(runtime_scratch_enablers(manifest).is_empty());
}

#[test]
fn a_runtime_dependency_without_the_feature_is_not_an_enabler() {
    let manifest = "[dependencies]\ncellgov_testkit = { path = \"..\" }\n";
    assert!(runtime_scratch_enablers(manifest).is_empty());
}

#[test]
fn the_feature_named_only_in_a_manifest_comment_is_not_an_enabler() {
    let manifest =
        "[dependencies]\n# scratch stays a dev-dependency\ncellgov_testkit = { path = \"..\" }\n";
    assert!(runtime_scratch_enablers(manifest).is_empty());
}

#[test]
fn a_scratch_path_outside_the_helper_is_a_call_site() {
    let source = "fn f() {\n    let d = std::env::temp_dir().join(\"cellgov_fixed\");\n}\n";
    assert_eq!(call_sites(source), vec![2]);
}

#[test]
fn a_tail_expression_scratch_path_is_a_call_site() {
    let source = "fn base() -> PathBuf {\n    std::env::temp_dir().join(\"cellgov_fixed\")\n}\n";
    assert_eq!(call_sites(source), vec![2]);
}

#[test]
fn temp_dir_named_only_in_prose_is_not_a_call_site() {
    let source = "/// A temp directory under `std::env::temp_dir()`.\npub struct TempDir;\n";
    assert!(call_sites(source).is_empty());
}

#[test]
fn temp_dir_named_only_in_a_string_is_not_a_call_site() {
    let source = "fn f() {\n    let s = \"std::env::temp_dir()\";\n}\n";
    assert!(call_sites(source).is_empty());
}

#[test]
fn a_helper_named_after_temp_dir_is_not_the_std_call() {
    let source = "fn fresh_temp_dir() -> PathBuf {\n    std::env::temp_dir().join(\"x\")\n}\n";
    // The `fn` name is skipped; the std call on line 2 is still counted.
    assert_eq!(call_sites(source), vec![2]);
}

#[test]
fn a_lifetime_does_not_swallow_the_following_code() {
    let source = "fn f<'a>(x: &'a str) -> Cow<'a, str> {\n    let d = std::env::temp_dir().join(\"cellgov_fixed\");\n    let _ = (x, d);\n    Cow::Borrowed(x)\n}\n";
    assert_eq!(call_sites(source), vec![2]);
}

/// A raw string may hold an unbalanced quote.
#[test]
fn a_raw_string_does_not_swallow_the_following_code() {
    let source =
        "fn f() {\n    let s = r#\"a \" b\"#;\n    let d = std::env::temp_dir().join(s);\n}\n";
    assert_eq!(call_sites(source), vec![3]);
}

#[test]
fn masking_preserves_byte_offsets_and_lines() {
    let source =
        "let s = \"aaa\";\n// bbb\nlet d = std::env::temp_dir().join(\"cellgov_fixed\");\n";
    let masked = mask_comments_and_literals(source);
    assert_eq!(masked.len(), source.len());
    assert_eq!(masked.matches('\n').count(), source.matches('\n').count());
    assert_eq!(call_sites(source), vec![3]);
}
