//! Convention guard: the runtime's whole-state mutable accessors stay
//! inside `cellgov_core`.
//!
//! An execution unit reaches guest memory only through the commit
//! pipeline. A host caller reaches it through `Runtime::host_write` or
//! `Runtime::place_bytes`. A `&mut GuestMemory`, `&mut UnitRegistry`
//! or `&mut ReservationTable` over live state is a fourth way in. It
//! skips the validation, the reservation clear sweep and the trace
//! record the three named paths carry. The space-scoped accessors hand
//! out the same handle behind one more argument, so this guard covers
//! them too.

use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, TokenStream, TokenTree};

/// The accessors this guard keeps inside the crate: the file that
/// declares each one, its name, and the signature that carries the
/// handle.
const NARROWED: [(&str, &str, &str); 5] = [
    (ACCESSORS, "memory_mut", "(&mut self) -> &mut GuestMemory"),
    (
        ACCESSORS,
        "registry_mut",
        "(&mut self) -> &mut UnitRegistry",
    ),
    (
        ACCESSORS,
        "reservations_mut",
        "(&mut self) -> &mut cellgov_sync::ReservationTable",
    ),
    (
        SPACES,
        "space_memory_mut",
        "(&mut self, space: AddressSpaceId) -> Result<&mut GuestMemory, SpaceError>",
    ),
    (
        SPACES,
        "space_reservations_mut",
        "(&mut self, space: AddressSpaceId) -> Result<&mut ReservationTable, SpaceError>",
    ),
];

/// The declaration site of the three whole-state accessors.
const ACCESSORS: &str = "crates/cellgov_core/src/runtime/accessors.rs";

/// The declaration site of the two space-scoped accessors.
const SPACES: &str = "crates/cellgov_core/src/runtime/spaces.rs";

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
    NARROWED.iter().map(|(_, name, _)| *name).collect()
}

/// One `fn` declaration. `signature` is every token after the name, up
/// to the body.
struct Declaration {
    name: String,
    bare_pub: bool,
    signature: String,
}

/// Strips the whitespace and the trailing comma rustfmt adds, so a
/// declaration that outgrew one line still compares equal to the
/// table's one-line spelling.
fn normalized(signature: &str) -> String {
    let dense: String = signature.chars().filter(|c| !c.is_whitespace()).collect();
    dense.replace(",)", ")").replace(",>", ">")
}

/// Idents that may sit between a visibility and its `fn`.
const FN_MODIFIERS: [&str; 4] = ["const", "async", "unsafe", "extern"];

/// Records every `fn` declaration in `stream`, at any nesting depth.
///
/// A generic declaration keeps its generic list in the signature, so a
/// widened twin is a separate entry with its own visibility.
fn declarations(stream: TokenStream, out: &mut Vec<Declaration>) {
    let trees: Vec<TokenTree> = stream.into_iter().collect();
    for (i, tree) in trees.iter().enumerate() {
        if let TokenTree::Group(group) = tree {
            declarations(group.stream(), out);
            continue;
        }
        let TokenTree::Ident(keyword) = tree else {
            continue;
        };
        if keyword != "fn" {
            continue;
        }
        // A name has to follow, so the `fn(u32) -> u32` pointer type is
        // no declaration.
        let Some(TokenTree::Ident(name)) = trees.get(i + 1) else {
            continue;
        };
        let mut signature = String::new();
        for tail in &trees[i + 2..] {
            match tail {
                TokenTree::Group(body) if body.delimiter() == Delimiter::Brace => break,
                TokenTree::Punct(p) if p.as_char() == ';' => break,
                _ => signature.push_str(&tail.to_string()),
            }
        }
        // `extern "C"`, `const` and friends sit between the visibility
        // and the `fn`; step back over them before reading it.
        let mut before = i;
        while before > 0 {
            match &trees[before - 1] {
                TokenTree::Ident(m) if FN_MODIFIERS.contains(&m.to_string().as_str()) => {
                    before -= 1
                }
                TokenTree::Literal(_) => before -= 1,
                _ => break,
            }
        }
        out.push(Declaration {
            name: name.to_string(),
            bare_pub: before > 0 && matches!(&trees[before - 1], TokenTree::Ident(v) if v == "pub"),
            signature,
        });
    }
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
    rt.space_memory_mut(space).unwrap().install_region(b, n, l, p);
    rt.space_reservations_mut(space).unwrap().clear();
    rt.mailbox_registry_mut().register(4);
    rt.signal_registry_mut().register();
    host.prx_registry_mut().register(m);
    let _ = rt.space_memory(space);
    let _ = rt.space_reservations(space);
    // rt.memory_mut() named in a comment is no call.
    println!("rt.memory_mut() named in a string is no call");
}
"#;

#[test]
fn the_matcher_sees_every_call_spelling_and_leaves_the_near_misses_alone() {
    let found = method_calls(MATCHER_SAMPLE, "the matcher sample", &narrowed_names());
    let names: Vec<&str> = found.iter().map(|(name, _)| name.as_str()).collect();
    let expected = [
        "registry_mut",
        "memory_mut",
        "reservations_mut",
        "space_memory_mut",
        "space_reservations_mut",
    ];
    assert_eq!(names, expected, "matched {found:?}");
}

/// The narrowed declaration the table spells, and a widened twin that
/// hands the same handle out through a generic signature.
const DECLARATION_SAMPLE: &str = r#"
impl Runtime {
    #[cfg(test)]
    #[inline]
    pub(crate) fn space_memory_mut(
        &mut self,
        space: AddressSpaceId,
    ) -> Result<&mut GuestMemory, SpaceError> {
        unimplemented!()
    }

    pub fn space_memory_mut<'a>(
        &'a mut self,
        space: AddressSpaceId,
    ) -> Result<&'a mut GuestMemory, SpaceError> {
        unimplemented!()
    }
}
"#;

#[test]
fn the_walk_reads_the_visibility_of_a_widened_twin_as_well_as_the_narrowed_form() {
    let mut found = Vec::new();
    declarations(
        tokens_of(DECLARATION_SAMPLE, "the declaration sample"),
        &mut found,
    );
    let walked: Vec<&str> = found.iter().map(|d| d.name.as_str()).collect();
    let named: Vec<&Declaration> = found
        .iter()
        .filter(|d| d.name == "space_memory_mut")
        .collect();
    assert_eq!(named.len(), 2, "walked {walked:?}");

    let table = NARROWED
        .iter()
        .find(|entry| entry.1 == "space_memory_mut")
        .expect("the table still names the space accessor");
    assert_eq!(normalized(&named[0].signature), normalized(table.2));
    assert!(!named[0].bare_pub, "`pub(crate)` read as a bare `pub`");

    assert_ne!(normalized(&named[1].signature), normalized(table.2));
    assert!(named[1].bare_pub, "the twin's `pub` went unread");
}

#[test]
fn every_accessor_is_declared_inside_the_crate() {
    let root = workspace_root();
    for (file, name, signature) in NARROWED {
        let mut found = Vec::new();
        declarations(tokens_of(&read(&root.join(file)), file), &mut found);
        let named: Vec<&Declaration> = found.iter().filter(|d| d.name == name).collect();
        assert!(
            named
                .iter()
                .any(|d| normalized(&d.signature) == normalized(signature)),
            "{file} no longer declares `fn {name}{signature}`. Rename or move it \
             and this guard stops covering the handle it returns; update NARROWED \
             to name where it lives now. Found: {:?}",
            named.iter().map(|d| &d.signature).collect::<Vec<_>>()
        );
        // The loop covers every declaration of the name: a widened twin
        // under its own `cfg` returns the same handle through a
        // signature the table would not match.
        for declaration in named {
            assert!(
                !declaration.bare_pub,
                "{file} hands `{name}{}` out of the crate. Every write to \
                 guest-visible state goes through the commit pipeline or \
                 `Runtime::host_write`; a caller elsewhere with bytes of its own \
                 places them with `Runtime::place_bytes`, and one that wants a \
                 space of its own shape builds the memory and passes it to \
                 `Runtime::create_address_space_with`.",
                normalized(&declaration.signature)
            );
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
