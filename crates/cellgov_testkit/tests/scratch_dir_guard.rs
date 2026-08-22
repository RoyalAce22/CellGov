//! Convention guard: scratch directories carry the process id.
//!
//! `cargo test` and `cargo test --release` are separate processes over
//! one `std::env::temp_dir()`, so a scratch path fixed at compile time
//! resolves to the same directory in both and one process's
//! `remove_dir_all` races the other's writes.
//!
//! Every `std::env::temp_dir()` call under `crates/`, `apps/`, and
//! `bridges/` must therefore reach `std::process::id()` -- inline in
//! the same statement, or through a `let pid = std::process::id();`
//! binding written in that exact form earlier in the same function and
//! interpolated as `{pid}` inside that same statement. A pid reached
//! any other way (a differently spelled binding, a positional
//! `format!` argument, a name assembled over several statements) is
//! reported; the failure message names the two accepted forms.
//!
//! The scan reads code only: comment bodies and string/char-literal
//! contents are blanked before matching, so neither a `temp_dir()`
//! written in prose nor a `process::id()` written in a comment can
//! stand in for the call the rule is about.

use std::fs;
use std::path::{Path, PathBuf};

/// The `let` form a function may bind once and interpolate as `{pid}`.
const PID_BINDING: &str = "let pid = std::process::id();";

/// Closers that end the statement the guard reads around a match. `{`
/// and `}` are in the set because a scratch path is often the tail
/// expression of a helper, where no `;` follows the call at all.
const STATEMENT_ENDS: [char; 3] = [';', '{', '}'];

/// Byte offset of the end of the statement `masked[from..]` opens.
///
/// A closer counts only at `(`/`[` depth zero: a scratch name built
/// through a `match`, an `if`, or a closure inside the call's argument
/// list carries braces nested in parentheses.
fn statement_end(masked: &str, from: usize) -> usize {
    let mut depth = 0usize;
    for (off, ch) in masked[from..].char_indices() {
        match ch {
            '(' | '[' => depth += 1,
            ')' | ']' => depth = depth.saturating_sub(1),
            c if depth == 0 && STATEMENT_ENDS.contains(&c) => return from + off,
            _ => {}
        }
    }
    masked.len()
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

/// Byte offset of the `fn` keyword opening the innermost item start
/// before `at`, or 0 when the call sits outside any function.
fn enclosing_fn_start(masked: &str, at: usize) -> usize {
    masked[..at]
        .match_indices("fn ")
        .filter(|(i, _)| *i == 0 || !is_ident_byte(masked.as_bytes()[i - 1]))
        .last()
        .map_or(0, |(i, _)| i)
}

/// Lines (1-indexed) of the `temp_dir()` calls in `source` whose scratch
/// path does not reach `std::process::id()`.
fn violation_lines(source: &str) -> Vec<usize> {
    let masked = mask_comments_and_literals(source);
    let mut lines = Vec::new();
    for (at, _) in masked.match_indices("temp_dir()") {
        // `fn fresh_temp_dir()` and calls to it are not the std call.
        if at > 0 && is_ident_byte(masked.as_bytes()[at - 1]) {
            continue;
        }
        let end = statement_end(&masked, at);
        let binds_pid = masked[enclosing_fn_start(&masked, at)..at].contains(PID_BINDING);
        // The `{pid}` marker lives inside a format string, so it is read
        // from the unmasked source; the offsets line up byte for byte.
        let discriminated = masked[at..end].contains("process::id()")
            || (binds_pid && source[at..end].contains("{pid}"));
        if !discriminated {
            lines.push(line_of(source, at));
        }
    }
    lines
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

#[test]
fn scratch_paths_carry_the_process_id() {
    let files = scanned_rs_files(&workspace_root());
    assert!(
        !files.is_empty(),
        "no .rs files found under crates/apps/bridges"
    );

    let mut violations = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        for line in violation_lines(&source) {
            violations.push((file.clone(), line));
        }
    }
    violations.sort();

    let mut report = String::new();
    for (file, line) in &violations {
        report.push_str(&format!("  {}:{line}\n", file.display()));
    }
    assert!(
        violations.is_empty(),
        "scratch paths fixed at compile time (two concurrent `cargo test` \
         invocations would share them -- put `std::process::id()` in the \
         same statement, or bind `{PID_BINDING}` earlier in the same \
         function and interpolate it as {{pid}}):\n{report}"
    );
}

#[test]
fn a_fixed_scratch_name_is_a_violation() {
    let source = "fn f() {\n    let d = std::env::temp_dir().join(\"cellgov_fixed\");\n}\n";
    assert_eq!(violation_lines(source), vec![2]);
}

#[test]
fn an_inline_process_id_satisfies_the_rule() {
    let source = "fn f() {\n    let d = std::env::temp_dir()\n        .join(format!(\"cellgov_{}\", std::process::id()));\n}\n";
    assert!(violation_lines(source).is_empty());
}

#[test]
fn a_pid_binding_in_the_same_function_satisfies_the_rule() {
    let source = "fn f(name: &str) {\n    let pid = std::process::id();\n    let d = std::env::temp_dir().join(format!(\"cellgov_{name}_{pid}\"));\n}\n";
    assert!(violation_lines(source).is_empty());
}

#[test]
fn a_pid_binding_in_another_function_does_not_satisfy_the_rule() {
    let source = "fn a() {\n    let pid = std::process::id();\n    let _ = pid;\n}\n\nfn b(pid: u32) {\n    let d = std::env::temp_dir().join(format!(\"cellgov_{pid}\"));\n}\n";
    assert_eq!(violation_lines(source), vec![7]);
}

#[test]
fn a_pid_interpolated_inside_a_braced_arm_satisfies_the_rule() {
    let source = "fn f(k: u8) {\n    let pid = std::process::id();\n    let d = std::env::temp_dir().join(match k {\n        0 => format!(\"a_{pid}\"),\n        _ => format!(\"b_{pid}\"),\n    });\n}\n";
    assert!(violation_lines(source).is_empty());
}

#[test]
fn an_inline_process_id_inside_a_closure_argument_satisfies_the_rule() {
    let source = "fn f() {\n    let d = std::env::temp_dir().join((|| {\n        format!(\"cellgov_{}\", std::process::id())\n    })());\n}\n";
    assert!(violation_lines(source).is_empty());
}

#[test]
fn a_tail_expression_scratch_path_is_a_violation() {
    let source = "fn base() -> PathBuf {\n    std::env::temp_dir().join(\"cellgov_fixed\")\n}\n\nfn other() {\n    let _ = std::process::id();\n}\n";
    assert_eq!(violation_lines(source), vec![2]);
}

#[test]
fn a_comment_naming_process_id_does_not_satisfy_the_rule() {
    let source = "fn f() {\n    let d = std::env::temp_dir() // process::id() would go here\n        .join(\"cellgov_fixed\");\n}\n";
    assert_eq!(violation_lines(source), vec![2]);
}

#[test]
fn a_string_literal_naming_process_id_does_not_satisfy_the_rule() {
    let source = "fn f() {\n    let d = std::env::temp_dir().join(\"cellgov_process::id()\");\n}\n";
    assert_eq!(violation_lines(source), vec![2]);
}

#[test]
fn temp_dir_named_only_in_prose_is_not_a_call_site() {
    let source = "/// A temp directory under `std::env::temp_dir()`.\npub struct TempDir;\n";
    assert!(violation_lines(source).is_empty());
}

#[test]
fn a_helper_named_after_temp_dir_is_not_the_std_call() {
    let source = "fn fresh_temp_dir() -> PathBuf {\n    let pid = std::process::id();\n    std::env::temp_dir().join(format!(\"cellgov_{pid}\"))\n}\n";
    assert!(violation_lines(source).is_empty());
}

#[test]
fn a_lifetime_does_not_swallow_the_following_code() {
    let source = "fn f<'a>(x: &'a str) -> Cow<'a, str> {\n    let d = std::env::temp_dir().join(\"cellgov_fixed\");\n    let _ = (x, d);\n    Cow::Borrowed(x)\n}\n";
    assert_eq!(violation_lines(source), vec![2]);
}

#[test]
fn masking_preserves_byte_offsets_and_lines() {
    let source =
        "let s = \"aaa\";\n// bbb\nlet d = std::env::temp_dir().join(\"cellgov_fixed\");\n";
    let masked = mask_comments_and_literals(source);
    assert_eq!(masked.len(), source.len());
    assert_eq!(masked.matches('\n').count(), source.matches('\n').count());
    assert_eq!(violation_lines(source), vec![3]);
}
