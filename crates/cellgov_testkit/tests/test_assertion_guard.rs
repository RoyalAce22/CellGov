//! Convention guards on the shapes of a test that cannot fail.
//!
//! - `#[should_panic]` names the panic it expects. Without `expected`
//!   the test passes on any panic, including one from its own setup.
//! - `#[ignore]` carries a reason. A bare one hides a test with no
//!   record of why.
//! - `assert!(x.is_err())` is never a test's only assertion: a
//!   rejection always carries which error, and a test that does not
//!   name it passes on the wrong one. An `is_ok()` alone is allowed,
//!   since an `Ok(())` acceptance carries nothing more to assert; where
//!   the `Ok` carries a value, the reader's audit still applies.
//! - A test does not sleep for a literal duration. A wait that orders
//!   two operations by wall clock is a flake on a loaded runner; a wait
//!   that must exist takes a named duration whose doc says what it is
//!   sized against.

use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, TokenStream, TokenTree};

/// Floor on the tests the guard inspects. The workspace carries a few
/// thousand; a collapse below this means the parser stopped
/// recognising `#[test]`.
const MIN_TESTS: usize = 1500;

#[derive(Debug, PartialEq, Eq)]
enum Fault {
    /// `#[should_panic]` with no `expected = "..."`.
    ShouldPanicWithoutExpected,
    /// `#[ignore]` with no reason string.
    IgnoreWithoutReason,
    /// The body's only assertion is `is_err()`.
    ErrorOnlyAssertion,
    /// `thread::sleep(Duration::from_*(literal))` in the body.
    LiteralSleep,
}

fn is_test_fn(f: &syn::ItemFn) -> bool {
    f.attrs.iter().any(|a| a.path().is_ident("test"))
}

fn attribute_faults(attrs: &[syn::Attribute], faults: &mut Vec<Fault>) {
    for attr in attrs {
        if attr.path().is_ident("should_panic") {
            let has_expected = match &attr.meta {
                syn::Meta::List(list) => list.tokens.to_string().contains("expected"),
                _ => false,
            };
            if !has_expected {
                faults.push(Fault::ShouldPanicWithoutExpected);
            }
        }
        if attr.path().is_ident("ignore") && matches!(attr.meta, syn::Meta::Path(_)) {
            faults.push(Fault::IgnoreWithoutReason);
        }
    }
}

/// Assertion macros the body invokes, as `(name, argument tokens)`.
fn assertions(stream: TokenStream, out: &mut Vec<(String, TokenStream)>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (i, tt) in tokens.iter().enumerate() {
        match tt {
            TokenTree::Ident(id) => {
                let name = id.to_string();
                let is_assert = name.starts_with("assert")
                    || name.starts_with("debug_assert")
                    || name == "panic"
                    || name == "unreachable";
                let bang =
                    matches!(tokens.get(i + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!');
                if is_assert && bang {
                    if let Some(TokenTree::Group(g)) = tokens.get(i + 2) {
                        out.push((name, g.stream()));
                    }
                }
            }
            TokenTree::Group(g) => assertions(g.stream(), out),
            _ => {}
        }
    }
}

/// The probe when the argument tokens are `<expr>.is_ok()` or
/// `<expr>.is_err()` with at most a message after them.
fn result_probe(args: &TokenStream) -> Option<String> {
    let tokens: Vec<TokenTree> = args.clone().into_iter().collect();
    let first_arg: Vec<&TokenTree> = tokens
        .iter()
        .take_while(|t| !matches!(t, TokenTree::Punct(p) if p.as_char() == ','))
        .collect();
    let n = first_arg.len();
    if n < 3 {
        return None;
    }
    let empty_call = matches!(first_arg[n - 1], TokenTree::Group(g) if g.delimiter() == Delimiter::Parenthesis && g.stream().is_empty());
    let dot = matches!(first_arg[n - 3], TokenTree::Punct(p) if p.as_char() == '.');
    match first_arg[n - 2] {
        TokenTree::Ident(id) if empty_call && dot && (id == "is_ok" || id == "is_err") => {
            Some(id.to_string())
        }
        _ => None,
    }
}

/// Whether the body sleeps for a literal duration.
fn has_literal_sleep(stream: TokenStream) -> bool {
    fn walk(stream: TokenStream) -> bool {
        let tokens: Vec<TokenTree> = stream.into_iter().collect();
        for (i, tt) in tokens.iter().enumerate() {
            match tt {
                TokenTree::Ident(id) if id == "sleep" => {
                    if let Some(TokenTree::Group(g)) = tokens.get(i + 1) {
                        let inner: Vec<TokenTree> = g.stream().into_iter().collect();
                        let constructs_duration = inner
                            .iter()
                            .any(|t| matches!(t, TokenTree::Ident(d) if d.to_string().starts_with("from_")));
                        let has_literal = inner.iter().any(|t| {
                            matches!(t, TokenTree::Group(a) if a.stream().into_iter().any(|x| matches!(x, TokenTree::Literal(_))))
                        });
                        if constructs_duration && has_literal {
                            return true;
                        }
                    }
                }
                TokenTree::Group(g) if walk(g.stream()) => return true,
                _ => {}
            }
        }
        false
    }
    walk(stream)
}

/// The byte offset of a `(line, column)` position in `source`.
fn byte_offset(source: &str, at: proc_macro2::LineColumn) -> usize {
    let line_start = source
        .split_inclusive('\n')
        .take(at.line - 1)
        .map(str::len)
        .sum::<usize>();
    let line = &source[line_start..];
    let column_bytes = line
        .char_indices()
        .nth(at.column)
        .map_or(line.len(), |(i, _)| i);
    line_start + column_bytes
}

/// The tokens of a function body, re-read from the source text between
/// the block's braces.
fn body_tokens(source: &str, f: &syn::ItemFn) -> TokenStream {
    let span = f.block.brace_token.span.join();
    let (start, end) = (
        byte_offset(source, span.start()),
        byte_offset(source, span.end()),
    );
    source[start..end]
        .parse()
        .expect("a function body tokenizes")
}

fn body_faults(source: &str, f: &syn::ItemFn, faults: &mut Vec<Fault>) {
    let body = body_tokens(source, f);
    let mut found = Vec::new();
    assertions(body.clone(), &mut found);
    if found.len() == 1
        && found[0].0 == "assert"
        && result_probe(&found[0].1).as_deref() == Some("is_err")
    {
        faults.push(Fault::ErrorOnlyAssertion);
    }
    if has_literal_sleep(body) {
        faults.push(Fault::LiteralSleep);
    }
}

/// `(test name, faults)` for every test in `items`; the count is the
/// number of tests seen.
fn check_items(source: &str, items: &[syn::Item], out: &mut Vec<(String, Vec<Fault>)>) -> usize {
    let mut seen = 0;
    for item in items {
        match item {
            syn::Item::Fn(f) if is_test_fn(f) => {
                seen += 1;
                let mut faults = Vec::new();
                attribute_faults(&f.attrs, &mut faults);
                body_faults(source, f, &mut faults);
                if !faults.is_empty() {
                    out.push((f.sig.ident.to_string(), faults));
                }
            }
            syn::Item::Mod(m) => {
                if let Some((_, nested)) = &m.content {
                    seen += check_items(source, nested, out);
                }
            }
            _ => {}
        }
    }
    seen
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

fn rs_files_under(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let entry =
            entry.unwrap_or_else(|e| panic!("cannot read entry under {}: {e}", dir.display()));
        let path = entry.path();
        let kind = entry
            .file_type()
            .unwrap_or_else(|e| panic!("cannot stat {}: {e}", path.display()));
        if kind.is_symlink() {
            continue;
        }
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

/// The guards whose own subject is a rule over source text.
fn defines_a_rule(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with("_guard.rs"))
}

fn run(source: &str) -> (usize, Vec<(String, Vec<Fault>)>) {
    let file = syn::parse_file(source).expect("test source parses");
    let mut out = Vec::new();
    let seen = check_items(source, &file.items, &mut out);
    (seen, out)
}

#[test]
fn the_attribute_checks_accept_the_house_forms_and_reject_the_bare_ones() {
    let (seen, out) = run(r#"
        #[test] #[should_panic(expected = "index")] fn a() { f(); }
        #[test] #[ignore = "needs the corpus"] fn b() { assert_eq!(1, 1); }
        #[test] #[should_panic] fn c() { f(); }
        #[test] #[ignore] fn d() { assert_eq!(1, 1); }
    "#);
    assert_eq!(seen, 4);
    assert_eq!(
        out,
        vec![
            ("c".to_string(), vec![Fault::ShouldPanicWithoutExpected]),
            ("d".to_string(), vec![Fault::IgnoreWithoutReason]),
        ]
    );
}

#[test]
fn the_assertion_check_flags_a_lone_result_probe_and_nothing_else() {
    let (seen, out) = run(r#"
        #[test] fn a() { let r = f(); assert!(r.is_ok()); }
        #[test] fn b() { let r = f(); assert!(r.is_err(), "{r:?}"); }
        #[test] fn c() { let r = f(); assert!(r.is_err()); assert_eq!(r.unwrap_err(), E::X); }
        #[test] fn d() { assert!(f().is_ok()); assert_eq!(g(), 2); }
        #[test] fn e() { let r = f(); assert!(matches!(r, Err(E::X))); }
        #[test] fn g() { assert!(list.is_empty()); }
        #[test] fn h() { let v = f().expect("x"); assert_eq!(v, 3); }
        mod inner { #[test] fn i() { assert!(super::f().is_ok()); } }
    "#);
    assert_eq!(seen, 8);
    assert_eq!(
        out,
        vec![("b".to_string(), vec![Fault::ErrorOnlyAssertion])],
        "only the lone is_err() probe is a fault; a lone is_ok() may be an Ok(()) acceptance"
    );
}

#[test]
fn the_sleep_check_flags_a_literal_duration_and_accepts_a_named_one() {
    let (_, out) = run(r#"
        #[test] fn a() { std::thread::sleep(Duration::from_millis(350)); assert_eq!(1, 1); }
        #[test] fn b() { thread::sleep(POLL_INTERVAL); assert_eq!(1, 1); }
        #[test] fn c() { let t = std::thread::spawn(move || { sleep(Duration::from_secs(1)); }); assert_eq!(1, 1); }
        #[test] fn d() { retry(op, std::thread::sleep, is_transient); assert_eq!(1, 1); }
        #[test] fn e() { assert_eq!(Duration::from_millis(5), d); }
    "#);
    assert_eq!(
        out,
        vec![
            ("a".to_string(), vec![Fault::LiteralSleep]),
            ("c".to_string(), vec![Fault::LiteralSleep]),
        ]
    );
}

#[test]
fn every_test_can_fail_for_the_reason_its_name_gives() {
    let root = workspace_root();
    let mut files = Vec::new();
    for group in ["crates", "apps", "bridges"] {
        rs_files_under(&root.join(group), &mut files);
    }
    files.sort();
    let mut seen = 0usize;
    let mut report = Vec::new();
    for file in &files {
        if defines_a_rule(file) {
            continue;
        }
        let source = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        let ast = syn::parse_file(&source)
            .unwrap_or_else(|e| panic!("cannot parse {}: {e}", file.display()));
        let mut out = Vec::new();
        seen += check_items(&source, &ast.items, &mut out);
        let shown = file.strip_prefix(&root).unwrap_or(file);
        for (name, faults) in out {
            report.push(format!("  {}  {name}  {faults:?}\n", shown.display()));
        }
    }
    assert!(
        seen >= MIN_TESTS,
        "gate went vacuous: only {seen} test(s) recognised across {} files",
        files.len()
    );
    report.sort();
    assert!(
        report.is_empty(),
        "{} test(s) carry a shape that cannot fail for its stated reason: name the expected \
         panic, give #[ignore] a reason, assert which error rather than is_err() alone, and \
         size any wait by a named duration:\n{}",
        report.len(),
        report.concat()
    );
}
