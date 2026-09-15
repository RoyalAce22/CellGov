//! Convention guard: the runtime's three whole-state mutable
//! accessors stay inside `cellgov_core`.
//!
//! An execution unit reaches guest memory only through the commit
//! pipeline. A host caller reaches it through `Runtime::host_write` or
//! `Runtime::place_bytes`. A `&mut GuestMemory`, `&mut UnitRegistry`
//! or `&mut ReservationTable` over the live boot state is a fourth way
//! in: it skips the validation, the reservation clear sweep and the
//! trace record the three named paths carry.
//!
//! `Runtime::space_memory_mut` and `Runtime::space_reservations_mut`
//! stay public and sit outside this guard: each names one address
//! space, and a caller that builds a space installs its regions and
//! its image through them before anything runs in it.

use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, TokenStream, TokenTree};

/// The accessors this guard keeps inside the crate, and the handle
/// each returns.
const NARROWED: [(&str, &str); 3] = [
    ("memory_mut", "&mut GuestMemory"),
    ("registry_mut", "&mut UnitRegistry"),
    ("reservations_mut", "&mut cellgov_sync::ReservationTable"),
];

/// The declaration site of all three.
const ACCESSORS: &str = "crates/cellgov_core/src/runtime/accessors.rs";

const OWNING_CRATE: &str = "crates/cellgov_core/";

/// Floor on the population the call-site scan walks. A scan below the
/// floor walked nothing, and reports the same empty violation list as
/// a clean tree.
const MIN_SOURCES: usize = 200;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

/// Every IO error panics with its path: a swallowed one shrinks the
/// set the guard inspects.
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

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for group in ["crates", "apps", "bridges"] {
        rs_files_under(&root.join(group), &mut found);
    }
    found
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn tokens_of(source: &str, label: &str) -> TokenStream {
    match source.parse() {
        Ok(stream) => stream,
        Err(e) => panic!("cannot tokenize {label}: {e}"),
    }
}

/// Records every `.<name>(..)` in `stream` whose name is in `names`,
/// with the line its dot sits on.
///
/// The match is on the ident behind the dot, so a sibling accessor
/// that ends in the same word -- `mailbox_registry_mut`,
/// `signal_registry_mut`, `prx_registry_mut` -- does not match. Those
/// three hand out registries the effects rule does not cover. The
/// receiver may sit on an earlier line, so a chain that rustfmt split
/// still matches.
fn walk(stream: TokenStream, names: &[&str], out: &mut Vec<(String, usize)>) {
    let trees: Vec<TokenTree> = stream.into_iter().collect();
    for (i, tree) in trees.iter().enumerate() {
        if let TokenTree::Group(group) = tree {
            walk(group.stream(), names, out);
            continue;
        }
        let TokenTree::Punct(dot) = tree else {
            continue;
        };
        if dot.as_char() != '.' {
            continue;
        }
        let Some(TokenTree::Ident(ident)) = trees.get(i + 1) else {
            continue;
        };
        let name = ident.to_string();
        if !names.contains(&name.as_str()) {
            continue;
        }
        let called = match trees.get(i + 2) {
            Some(TokenTree::Group(args)) => args.delimiter() == Delimiter::Parenthesis,
            _ => false,
        };
        if called {
            out.push((name, dot.span().start().line));
        }
    }
}

/// Calls of `names` in `source`. The token stream carries no comment
/// and no string body, so a doc comment that names an accessor is no
/// call site.
fn method_calls(source: &str, label: &str, names: &[&str]) -> Vec<(String, usize)> {
    let mut out = Vec::new();
    walk(tokens_of(source, label), names, &mut out);
    out
}

fn narrowed_names() -> Vec<&'static str> {
    NARROWED.iter().map(|(name, _)| *name).collect()
}

/// Every call spelling the workspace produces, plus the near misses
/// the matcher must ignore.
const MATCHER_SAMPLE: &str = r#"
fn f(rt: &mut Runtime, host: &mut Lv2Host) {
    rt.registry_mut().register_with(g);
    let m = rt
        .memory_mut()
        .apply_commit(range, bytes);
    assert!(rt.reservations_mut().is_empty());
    rt.mailbox_registry_mut().register(4);
    rt.signal_registry_mut().register();
    host.prx_registry_mut().register(m);
    // rt.memory_mut() named in a comment is no call.
    println!("rt.memory_mut() named in a string is no call");
}
"#;

#[test]
fn the_matcher_sees_every_call_spelling_and_leaves_the_near_misses_alone() {
    let found = method_calls(MATCHER_SAMPLE, "the matcher sample", &narrowed_names());
    let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
    let expected = ["registry_mut", "memory_mut", "reservations_mut"];
    assert_eq!(names, expected, "matched {found:?}");
}

#[test]
fn the_three_accessors_are_declared_inside_the_crate() {
    let source = read(&workspace_root().join(ACCESSORS));
    for (name, handle) in NARROWED {
        let declaration = format!("fn {name}(&mut self) -> {handle}");
        let mut at = source.find(&declaration).unwrap_or_else(|| {
            panic!(
                "{ACCESSORS} no longer declares `{declaration}`. Rename or move it \
                 and this guard stops covering the handle it returns; update \
                 NARROWED to name where it lives now."
            )
        });
        // The loop covers every occurrence, because a widened twin
        // under its own `cfg` can sit behind the narrowed declaration.
        loop {
            let line_start = source[..at].rfind('\n').map_or(0, |nl| nl + 1);
            let declaration_line = &source[line_start..at + declaration.len()];
            assert!(
                !declaration_line.contains("pub fn"),
                "{ACCESSORS} hands `{handle}` out of the crate through `{name}`. \
                 Every write to guest-visible state goes through the commit \
                 pipeline or `Runtime::host_write`; a caller elsewhere with bytes \
                 of its own places them with `Runtime::place_bytes`."
            );
            let tail = at + declaration.len();
            match source[tail..].find(&declaration) {
                Some(next) => at = tail + next,
                None => break,
            }
        }
    }
}

#[test]
fn no_source_outside_the_crate_calls_them() {
    let root = workspace_root();
    let files = rust_sources(&root);
    assert!(
        files.len() >= MIN_SOURCES,
        "gate went vacuous: only {} source file(s) found, expected at least {MIN_SOURCES}",
        files.len()
    );

    let names = narrowed_names();
    let mut violations = Vec::new();
    for file in &files {
        let rel = file
            .strip_prefix(&root)
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/");
        if rel.starts_with(OWNING_CRATE) {
            continue;
        }
        for (name, line) in method_calls(&read(file), &rel, &names) {
            violations.push(format!("  {rel}:{line}: {name}\n"));
        }
    }
    violations.sort();
    assert!(
        violations.is_empty(),
        "a caller outside {OWNING_CRATE} reaches the runtime's guest-visible \
         state through a mutable handle:\n{}",
        violations.concat()
    );
}
