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

/// Number of `mod tests` declarations that satisfied the rule.
///
/// Tallied so the gate can prove it inspected something: a guard that
/// recognises no declaration at all reports the same empty violation
/// list as a conforming workspace.
fn check_items(
    items: &[syn::Item],
    file: &Path,
    violations: &mut Vec<(PathBuf, usize, String)>,
) -> usize {
    let mut conforming = 0;
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
            } else {
                conforming += 1;
            }
        }
        if let Some((_, nested)) = &m.content {
            conforming += check_items(nested, file, violations);
        }
    }
    conforming
}

/// Floor on the population the rule polices. Nearly every crate in the
/// workspace declares one, so a collapse to single digits means the
/// matcher stopped recognising the declaration.
const MIN_CONFORMING_MODULES: usize = 150;

/// Positive control: the two rejected spellings and the accepted one,
/// checked against the matcher directly rather than through an empty
/// violation list.
#[test]
fn the_matcher_separates_the_accepted_declaration_from_the_two_rejected_ones() {
    fn run(src: &str) -> (usize, Vec<String>) {
        let ast = syn::parse_file(src).expect("test source parses");
        let mut v = Vec::new();
        let n = check_items(&ast.items, Path::new("x.rs"), &mut v);
        (n, v.into_iter().map(|(_, _, what)| what).collect())
    }

    let (n, v) = run("#[cfg(test)]
#[path = \"tests/host_tests.rs\"]
mod tests;
");
    assert_eq!((n, v.len()), (1, 0));

    let (n, v) = run("#[cfg(test)]
mod tests {
    fn f() {}
}
");
    assert_eq!(n, 0);
    assert_eq!(v, vec!["inline `mod tests { }` body".to_string()]);

    let (n, v) = run("#[cfg(test)]
mod tests;
");
    assert_eq!(n, 0);
    assert_eq!(v, vec!["bare `mod tests;` without #[path]".to_string()]);

    let (n, v) = run("mod other;
");
    assert_eq!((n, v.len()), (0, 0), "only a module named tests is policed");
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
    let mut conforming = 0usize;
    for file in &files {
        let source = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        match syn::parse_file(&source) {
            Ok(ast) => conforming += check_items(&ast.items, file, &mut violations),
            Err(e) => violations.push((file.clone(), 0, format!("parse error: {e}"))),
        }
    }
    violations.sort();
    assert!(
        conforming >= MIN_CONFORMING_MODULES,
        "gate went vacuous: only {conforming} conforming `mod tests`          declaration(s) recognised across {} source files, expected at least          {MIN_CONFORMING_MODULES}",
        files.len()
    );

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
