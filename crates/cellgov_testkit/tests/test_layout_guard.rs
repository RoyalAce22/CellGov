//! Convention guard: unit-test modules live in external files.
//!
//! Under `crates/*/src`, `apps/*/src`, and `bridges/*/src`, a module
//! named `tests` is declared as `#[path = "..."] mod tests;`, and no
//! module under a `#[cfg(test)]` attribute carries an inline body. An
//! inline `mod name { }` body under `cfg(test)`, and a bare
//! `mod tests;`, both fail this test.
//!
//! Three marks reach a module, so that no spelling of a test module
//! escapes: the name `tests`, a `#[cfg(test)]` attribute, or a body
//! that declares a `#[test]` function. The third mark also reads the
//! file itself, because a `#[test]` function at file scope sits in no
//! module.
//!
//! The third mark stops at the external test bodies. A file in a
//! `tests` directory is already where the rule asked the tests to go.

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

/// Whether the attribute is a `#[cfg(...)]` whose predicate names `test`.
fn is_cfg_test(attr: &syn::Attribute) -> bool {
    let syn::Meta::List(list) = &attr.meta else {
        return false;
    };
    list.path.is_ident("cfg")
        && list
            .tokens
            .to_string()
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|word| word == "test")
}

/// Whether the items declare a function under a `test`-suffixed
/// attribute, which `#[tokio::test]` satisfies as well as `#[test]`.
fn declares_test_fn(items: &[syn::Item]) -> bool {
    items.iter().any(|item| {
        let syn::Item::Fn(f) = item else { return false };
        f.attrs.iter().any(|attr| {
            attr.path()
                .segments
                .last()
                .is_some_and(|seg| seg.ident == "test")
        })
    })
}

/// Number of test-module declarations that satisfied the rule.
///
/// Tallied so the gate can prove it inspected something: a guard that
/// recognises no declaration at all reports the same empty violation
/// list as a conforming workspace.
fn check_items(
    items: &[syn::Item],
    file: &Path,
    test_body: bool,
    violations: &mut Vec<(PathBuf, usize, String)>,
) -> usize {
    let mut conforming = 0;
    for item in items {
        let syn::Item::Mod(m) = item else { continue };
        let named_tests = m.ident == "tests";
        let declares_tests = !test_body
            && m.content
                .as_ref()
                .is_some_and(|(_, nested)| declares_test_fn(nested));
        if named_tests || declares_tests || m.attrs.iter().any(is_cfg_test) {
            let line = m.ident.span().start().line;
            let name = m.ident.to_string();
            if m.content.is_some() {
                violations.push((
                    file.to_path_buf(),
                    line,
                    format!("inline `mod {name} {{ }}` body"),
                ));
            } else if named_tests && !m.attrs.iter().any(|a| a.path().is_ident("path")) {
                // A cfg(test) helper module (`mod test_support;`) lives
                // in its own file already; only `tests` needs the
                // `#[path]` that keeps it out of a `tests.rs` sibling.
                violations.push((
                    file.to_path_buf(),
                    line,
                    format!("bare `mod {name};` without #[path]"),
                ));
            } else {
                conforming += 1;
            }
        }
        if let Some((_, nested)) = &m.content {
            conforming += check_items(nested, file, test_body, violations);
        }
    }
    conforming
}

/// Everything the rule asks of one file: the file-scope mark, then
/// every module it declares.
///
/// `check_items` walks modules, so a `#[test]` function at file scope
/// would otherwise pass unread.
fn check_source(
    source: &str,
    file: &Path,
    test_body: bool,
    violations: &mut Vec<(PathBuf, usize, String)>,
) -> usize {
    let ast = match syn::parse_file(source) {
        Ok(ast) => ast,
        Err(e) => {
            violations.push((file.to_path_buf(), 0, format!("parse error: {e}")));
            return 0;
        }
    };
    if !test_body && declares_test_fn(&ast.items) {
        violations.push((
            file.to_path_buf(),
            0,
            "`#[test]` function at file scope, outside any module".to_string(),
        ));
    }
    check_items(&ast.items, file, test_body, violations)
}

/// Whether the file is one of the external test bodies, which the
/// workspace keeps in a `tests` directory beside the source it covers.
fn is_external_test_body(file: &Path) -> bool {
    file.parent()
        .is_some_and(|dir| dir.file_name().is_some_and(|name| name == "tests"))
}

/// Floor on the population the rule polices. Nearly every crate in the
/// workspace declares one, so a collapse to single digits means the
/// matcher stopped recognising the declaration.
const MIN_CONFORMING_MODULES: usize = 150;

/// Floor on the external test bodies the third mark exempts. A
/// collapse here means the sweep no longer finds them, and reports
/// every grouping module inside them.
const MIN_TEST_BODIES: usize = 100;

/// Positive control: each rejected spelling and each accepted one,
/// checked against the matcher directly rather than through an empty
/// violation list.
#[test]
fn the_matcher_separates_the_accepted_declarations_from_the_rejected_ones() {
    fn scan(src: &str, test_body: bool) -> (usize, Vec<String>) {
        let ast = syn::parse_file(src).expect("test source parses");
        let mut v = Vec::new();
        let n = check_items(&ast.items, Path::new("x.rs"), test_body, &mut v);
        (n, v.into_iter().map(|(_, _, what)| what).collect())
    }
    let run = |src: &str| scan(src, false);

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
    assert_eq!(
        (n, v.len()),
        (0, 0),
        "a module that is neither named tests, nor under cfg(test), nor \
         a declarer of #[test] functions is not policed"
    );

    let (n, v) = run("mod other {
    fn f() {}
}
");
    assert_eq!(
        (n, v.len()),
        (0, 0),
        "an inline module declaring no #[test] function is not policed"
    );

    let (n, v) = run("mod prefix_tests {
    #[test]
    fn f() {}
}
");
    assert_eq!(n, 0);
    assert_eq!(
        v,
        vec!["inline `mod prefix_tests { }` body".to_string()],
        "an inline module holding a #[test] function is refused without \
         a cfg(test) attribute to mark it"
    );

    let (n, v) = run("#[cfg(feature = \"x\")]
mod gated {
    #[tokio::test]
    async fn f() {}
}
");
    assert_eq!(n, 0);
    assert_eq!(
        v,
        vec!["inline `mod gated { }` body".to_string()],
        "a path-qualified test attribute marks the module too"
    );

    let (n, v) = run("#[cfg(test)]
mod space_batch_tests {
    fn f() {}
}
");
    assert_eq!(n, 0);
    assert_eq!(
        v,
        vec!["inline `mod space_batch_tests { }` body".to_string()]
    );

    let (n, v) = run("#[cfg(test)]
#[path = \"tests/other_tests.rs\"]
mod other_tests;
#[cfg(all(test, feature = \"x\"))]
#[path = \"tests/gated_tests.rs\"]
mod gated_tests;
#[cfg(test)]
mod test_support;
#[cfg(feature = \"x\")]
mod not_a_test_module {
    fn f() {}
}
");
    assert_eq!(
        (n, v.len()),
        (3, 0),
        "a cfg(test) helper in its own file conforms without #[path]"
    );

    let grouped = "mod prefix_tests {
    #[test]
    fn f() {}
}
";
    let (n, v) = scan(grouped, true);
    assert_eq!(
        (n, v.len()),
        (0, 0),
        "inside an external test body the same module groups cases and is not policed"
    );
    let (_, v) = scan(grouped, false);
    assert_eq!(v.len(), 1, "the same source outside a test body is refused");
}

/// Positive control for the two predicates the sweep reads directly,
/// which `check_items` never sees: the file-scope mark and the
/// exemption that decides where the third mark applies.
#[test]
fn the_file_scope_mark_and_the_test_body_exemption_separate_their_cases() {
    fn declares(src: &str) -> bool {
        declares_test_fn(&syn::parse_file(src).expect("test source parses").items)
    }

    assert!(
        declares("#[test]\nfn f() {}\n"),
        "a #[test] function at file scope is seen, though it sits in no module"
    );
    assert!(
        !declares("fn f() {}\nstruct S;\n"),
        "a file declaring no #[test] function is not marked"
    );

    fn sweep(src: &str, test_body: bool) -> Vec<String> {
        let mut v = Vec::new();
        check_source(src, Path::new("x.rs"), test_body, &mut v);
        v.into_iter().map(|(_, _, what)| what).collect()
    }

    assert_eq!(
        sweep("#[test]\nfn f() {}\n", false),
        vec!["`#[test]` function at file scope, outside any module".to_string()],
        "the sweep reads the file-scope mark, not only the modules"
    );
    assert!(
        sweep("#[test]\nfn f() {}\n", true).is_empty(),
        "an external test body is where a file-scope #[test] belongs"
    );
    assert_eq!(
        sweep("fn f() {}\n", false),
        Vec::<String>::new(),
        "a source file declaring no test is untouched"
    );
    // The wording is syn's and moves with the version; the shape is ours.
    let refused = sweep("fn f( {}\n", false);
    assert_eq!(refused.len(), 1);
    assert!(
        refused[0].starts_with("parse error: "),
        "a file the parser refuses is reported rather than skipped, got {:?}",
        refused[0]
    );

    assert!(is_external_test_body(Path::new(
        "crates/cellgov_lv2/src/host/tests/host_tests.rs"
    )));
    assert!(
        !is_external_test_body(Path::new("crates/cellgov_lv2/src/host/diagnostics.rs")),
        "a source file beside the tests directory is not one of the bodies"
    );
    assert!(
        !is_external_test_body(Path::new("crates/cellgov_lv2/src/tests.rs")),
        "the exemption reads the directory, not a file named tests"
    );
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
    let mut test_bodies = 0usize;
    for file in &files {
        let source = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let test_body = is_external_test_body(file);
        test_bodies += usize::from(test_body);
        conforming += check_source(&source, file, test_body, &mut violations);
    }
    assert!(
        test_bodies >= MIN_TEST_BODIES,
        "gate went vacuous: only {test_bodies} external test body file(s) recognised \
         across {} source files, expected at least {MIN_TEST_BODIES}",
        files.len()
    );
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
