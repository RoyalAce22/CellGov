//! Convention guard: unit-test modules live in external files.
//!
//! Every module named `tests` under `crates/*/src`, `apps/*/src`, and
//! `bridges/*/src` must be declared as `#[path = "..."] mod tests;`.
//! Inline `mod tests { }` bodies and bare `mod tests;` declarations
//! both fail this test.

use std::fs;
use std::path::{Path, PathBuf};

fn rs_files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("cannot read entry under {}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            rs_files_under(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

fn check_items(items: &[syn::Item], file: &Path, violations: &mut Vec<(PathBuf, usize, String)>) {
    for item in items {
        let syn::Item::Mod(m) = item else { continue };
        if m.ident == "tests" {
            let line = m.ident.span().start().line;
            if m.content.is_some() {
                violations.push((
                    file.to_path_buf(),
                    line,
                    "inline `mod tests { }` body".to_string(),
                ));
            } else if !m.attrs.iter().any(|a| a.path().is_ident("path")) {
                violations.push((
                    file.to_path_buf(),
                    line,
                    "bare `mod tests;` without #[path]".to_string(),
                ));
            }
        }
        if let Some((_, nested)) = &m.content {
            check_items(nested, file, violations);
        }
    }
}

#[test]
fn unit_test_modules_are_external_files() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf();

    let mut files = Vec::new();
    for group in ["crates", "apps", "bridges"] {
        let group_dir = workspace_root.join(group);
        let crate_dirs = fs::read_dir(&group_dir)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", group_dir.display()));
        for entry in crate_dirs {
            let src = entry
                .unwrap_or_else(|e| panic!("cannot read {group} entry: {e}"))
                .path()
                .join("src");
            if src.is_dir() {
                rs_files_under(&src, &mut files);
            }
        }
    }
    assert!(
        !files.is_empty(),
        "no .rs files found under crates/apps/bridges */src"
    );

    let mut violations = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        match syn::parse_file(&source) {
            Ok(ast) => check_items(&ast.items, file, &mut violations),
            Err(e) => violations.push((file.clone(), 0, format!("parse error: {e}"))),
        }
    }
    violations.sort();

    let mut report = String::new();
    for (file, line, what) in &violations {
        report.push_str(&format!("  {}:{line}  {what}\n", file.display()));
    }
    assert!(
        violations.is_empty(),
        "test-layout violations (declare as `#[cfg(test)] #[path = \"...\"] mod tests;` \
         with the body in an external file):\n{report}"
    );
}
