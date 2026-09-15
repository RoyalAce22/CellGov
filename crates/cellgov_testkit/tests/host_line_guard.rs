//! Convention guard: each workspace member sits on one side of the host line.
//!
//! The lists the compiler enforces live in the workspace
//! `clippy.toml`. They bind a crate only while its root forbids
//! `clippy::disallowed_methods` and `clippy::disallowed_macros`, since
//! the workspace lint table allows both everywhere else. No lint fires
//! in these cases, so this guard fails on each:
//!
//! - A runtime crate root drops the attribute.
//! - A new member sits on neither side.
//! - A runtime source keeps a `static` that is `mut`, or of an atomic,
//!   lock or cell type. A const constructor builds such a static, so
//!   no entry in the method list can catch it.
//! - A runtime source calls `include!`. The macro list does not reach
//!   it: an entry for it denies nothing.

use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{TokenStream, TokenTree};

/// The members below the host line. Outside their test builds, they use
/// no clock, environment, file, console or process-global state.
const RUNTIME: &[&str] = &[
    "crates/cellgov_ps3_abi",
    "crates/cellgov_time",
    "crates/cellgov_event",
    "crates/cellgov_mem",
    "crates/cellgov_effects",
    "crates/cellgov_sync",
    "crates/cellgov_dma",
    "crates/cellgov_exec",
    "crates/cellgov_trace",
    "crates/cellgov_core",
    "crates/cellgov_lv2",
    "crates/cellgov_spu",
    "crates/cellgov_ppu",
];

/// The members above it, which keep their host access.
const HOST: &[&str] = &[
    "crates/cellgov_compare",
    "crates/cellgov_boot",
    "crates/cellgov_testkit",
    "crates/cellgov_explore",
    "crates/cellgov_terminal",
    "apps/cellgov_cli",
    "apps/cellgov_mkelf",
    "apps/cellgov_install",
    "bridges/rpcs3_to_observation",
];

/// The arguments of the crate-root `cfg_attr`, whitespace removed.
const FORBID_ARGS: &str = "not(test),forbid(clippy::disallowed_methods,clippy::disallowed_macros,clippy::print_stdout,clippy::print_stderr,clippy::dbg_macro)";

/// Host reads the workspace `clippy.toml` must keep denying, at least
/// one per family.
const REQUIRED_METHODS: &[&str] = &[
    "std::time::Instant::now",
    "std::time::SystemTime::now",
    "std::time::SystemTime::elapsed",
    "std::thread::sleep",
    "std::env::var",
    "std::env::var_os",
    "std::env::args",
    "std::fs::read",
    "std::fs::write",
    "std::fs::File::open",
    "std::fs::File::create",
    "std::path::Path::exists",
    "std::io::stdin",
    "std::io::stdout",
    "std::io::stderr",
    "std::process::Command::new",
    "std::process::exit",
    "std::sync::OnceLock::new",
    "std::sync::LazyLock::new",
    "std::thread::LocalKey::with",
    "std::thread::spawn",
    "std::thread::scope",
    "std::thread::Builder::spawn",
    "std::thread::current",
    "std::net::TcpStream::connect",
    "std::net::TcpListener::bind",
    "std::net::UdpSocket::bind",
    "std::hash::RandomState::new",
];

/// Compile-time host reads the workspace `clippy.toml` must keep
/// denying. These read the build machine, so no observer can supply the
/// answer and no refusal can name it.
const REQUIRED_MACROS: &[&str] = &[
    "std::env",
    "std::option_env",
    "std::include_str",
    "std::include_bytes",
];

/// Collections the workspace `clippy.toml` must keep denying, and the
/// entropy hasher behind them, which `default()` reaches through a
/// trait impl no method entry can name.
const REQUIRED_TYPES: &[&str] = &[
    "std::collections::HashMap",
    "std::collections::HashSet",
    "std::hash::RandomState",
];

/// Type names that make a `static` a store the program changes at run
/// time, besides every `Atomic*`.
const INTERIOR_MUTABLE: &[&str] = &[
    "Mutex", "RwLock", "Condvar", "Once", "OnceLock", "LazyLock", "Cell", "RefCell", "OnceCell",
    "LazyCell",
];

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// `text` with each `#` comment cut; a `#` inside a quoted string stays.
fn without_toml_comments(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        let mut quoted = false;
        let mut escaped = false;
        let mut end = line.len();
        for (i, c) in line.char_indices() {
            if escaped {
                escaped = false;
            } else if quoted && c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = !quoted;
            } else if c == '#' && !quoted {
                end = i;
                break;
            }
        }
        out.push('\n');
        out.push_str(&line[..end]);
    }
    out
}

/// The quoted entries of the `[workspace] members = [ ... ]` array.
fn members(manifest: &str) -> Vec<String> {
    let manifest = without_toml_comments(manifest);
    let Some(start) = manifest.find("\nmembers = [") else {
        return Vec::new();
    };
    let body = &manifest[start + "\nmembers = [".len()..];
    let body = &body[..body.find(']').unwrap_or(body.len())];
    body.split(',')
        .map(|entry| entry.trim().trim_matches('"').to_string())
        .filter(|entry| !entry.is_empty())
        .collect()
}

fn without_whitespace(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// The `path = "..."` values of one `clippy.toml` list.
fn listed_paths(clippy_toml: &str, list: &str) -> Vec<String> {
    let clippy_toml = without_toml_comments(clippy_toml);
    let Some(start) = clippy_toml.find(&format!("\n{list} = [")) else {
        return Vec::new();
    };
    let body = &clippy_toml[start..];
    let body = &body[..body.find("\n]").unwrap_or(body.len())];
    body.split("path = \"")
        .skip(1)
        .filter_map(|rest| rest.split('"').next())
        .map(str::to_string)
        .collect()
}

/// Whether `attr` is the inner `cfg_attr(not(test), forbid(...))` this
/// guard pins.
fn is_host_line_forbid(attr: &syn::Attribute) -> bool {
    let syn::Meta::List(list) = &attr.meta else {
        return false;
    };
    matches!(attr.style, syn::AttrStyle::Inner(_))
        && list.path.is_ident("cfg_attr")
        && without_whitespace(&list.tokens.to_string()) == FORBID_ARGS
}

/// Whether the inner attributes of `root` itself include that attribute.
fn forbids_host_access(root: &syn::File) -> bool {
    root.attrs.iter().any(is_host_line_forbid)
}

/// Crate roots a member carries besides `src/lib.rs`: a binary target, a
/// build script, or a `[lib]` table that moves the library root.
///
/// Cargo compiles a build script as its own crate and runs it on the
/// build machine, so the attribute on `src/lib.rs` does not reach it and
/// the workspace lint table leaves both host-line lints allowed there.
fn other_crate_roots(member_dir: &Path) -> Vec<String> {
    let mut found: Vec<String> = ["src/main.rs", "src/bin", "build.rs"]
        .into_iter()
        .filter(|rel| member_dir.join(rel).exists())
        .map(str::to_string)
        .collect();
    let manifest = without_toml_comments(&read(&member_dir.join("Cargo.toml")));
    let mut table = "";
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            table = line;
            if line == "[[bin]]" {
                found.push(line.to_string());
            }
        } else if table == "[lib]" && line.starts_with("path") {
            found.push(format!("[lib] {line}"));
        } else if table == "[package]" && line.starts_with("build") && line.contains('"') {
            // A quoted value moves the build script; `build = false`
            // turns it off and names no root.
            found.push(format!("[package] {line}"));
        }
    }
    found
}

fn is_punct(token: &TokenTree, ch: char) -> bool {
    matches!(token, TokenTree::Punct(p) if p.as_char() == ch)
}

fn push_idents(token: &TokenTree, out: &mut Vec<String>) {
    match token {
        TokenTree::Ident(ident) => out.push(ident.to_string()),
        TokenTree::Group(group) => {
            for inner in group.stream() {
                push_idents(&inner, out);
            }
        }
        TokenTree::Punct(_) | TokenTree::Literal(_) => {}
    }
}

/// Every `static` in `stream` that is `mut` or whose type names an
/// atomic, lock or cell, as its declaration up to the `=`.
fn mutable_statics(stream: TokenStream, found: &mut Vec<String>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (i, token) in tokens.iter().enumerate() {
        if let TokenTree::Group(group) = token {
            mutable_statics(group.stream(), found);
            continue;
        }
        let TokenTree::Ident(ident) = token else {
            continue;
        };
        // `'static` is a lifetime: a quote punct, then the ident.
        if ident != "static" || (i > 0 && is_punct(&tokens[i - 1], '\'')) {
            continue;
        }
        let decl: Vec<&TokenTree> = tokens[i + 1..]
            .iter()
            .take_while(|t| !is_punct(t, '=') && !is_punct(t, ';'))
            .collect();
        let is_mut = matches!(decl.first(), Some(TokenTree::Ident(m)) if m == "mut");
        let mut names = Vec::new();
        for t in decl.iter().skip_while(|t| !is_punct(t, ':')) {
            push_idents(t, &mut names);
        }
        let interior = names
            .iter()
            .any(|n| n.starts_with("Atomic") || INTERIOR_MUTABLE.contains(&n.as_str()));
        if is_mut || interior {
            let text: Vec<String> = decl.iter().map(|t| t.to_string()).collect();
            found.push(format!("static {}", text.join(" ")));
        }
    }
}

/// Every `include!` call in `stream`, as the call text.
///
/// `include!` reads the build machine's file system, which is what the
/// `clippy.toml` macro list refuses. The lint does not fire on this
/// one, so this scan is what refuses it.
fn include_calls(stream: TokenStream, found: &mut Vec<String>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (i, token) in tokens.iter().enumerate() {
        if let TokenTree::Group(group) = token {
            include_calls(group.stream(), found);
            continue;
        }
        let TokenTree::Ident(ident) = token else {
            continue;
        };
        if ident != "include" || !tokens.get(i + 1).is_some_and(|t| is_punct(t, '!')) {
            continue;
        }
        let arg = match tokens.get(i + 2) {
            Some(TokenTree::Group(g)) => g.stream().to_string(),
            _ => continue,
        };
        found.push(format!("include!({arg})"));
    }
}

/// The `.rs` files under `dir`, outside each `tests` directory.
///
/// The runtime crates keep each unit-test file under a `tests` directory.
fn non_test_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("cannot read entry under {}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            if path.file_name().is_some_and(|name| name != "tests") {
                non_test_sources(&path, out);
            }
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_parsers_read_members_and_listed_paths_and_nothing_else() {
    let manifest = "[workspace]\nmembers = [\n    \"crates/a\",\n    # \"crates/old\", [retired]\n    \"apps/b\", # trailing\n]\ndefault-members = [\n    \"crates/a\",\n]\n";
    assert_eq!(members(manifest), ["crates/a", "apps/b"]);
    let toml = "\nfoo = [\n    { path = \"x::Y\" },\n]\nbar = [\n    { path = \"a::b\", reason = \"r # s\" },\n    # { path = \"e::f\" },\n    { path = \"c::d\" }, # { path = \"g::h\" }\n]\n";
    assert_eq!(listed_paths(toml, "bar"), ["a::b", "c::d"]);
    assert!(listed_paths(toml, "baz").is_empty());
}

#[test]
fn the_attribute_check_reads_the_crate_level_attribute_and_nothing_else() {
    let check = |src: &str| forbids_host_access(&syn::parse_file(src).expect("control parses"));
    let attr = "#![cfg_attr(\n    not(test),\n    forbid(\n        clippy::disallowed_methods,\n        clippy::disallowed_macros,\n        clippy::print_stdout,\n        clippy::print_stderr,\n        clippy::dbg_macro\n    )\n)]\n";
    assert!(check(&format!(
        "//! Docs.\n\n#![deny(unused_must_use)]\n{attr}\npub mod a;\n"
    )));
    let commented = format!("// {}", attr.replace('\n', "\n// "));
    assert!(!check(&format!("{commented}\npub mod a;\n")));
    assert!(!check(&format!("pub mod a {{\n{attr}\n}}\n")));
    assert!(!check(
        "const S: &str = \"#![cfg_attr(not(test), forbid(clippy::disallowed_methods, \
         clippy::disallowed_macros, clippy::print_stdout, clippy::print_stderr, \
         clippy::dbg_macro))]\";\n"
    ));
    assert!(!check(
        "#![cfg_attr(not(test), forbid(clippy::disallowed_methods))]\n"
    ));
    assert!(
        !check(
            "#![cfg_attr(not(test), forbid(clippy::disallowed_methods, clippy::print_stdout, \
             clippy::print_stderr, clippy::dbg_macro))]\n"
        ),
        "a root that forbids the method lint but not the macro lint leaves the \
         compile-time host reads unrefused"
    );
}

#[test]
fn the_static_scan_flags_atomic_lock_cell_and_mut_statics_and_nothing_else() {
    let scan = |src: &str| {
        let mut found = Vec::new();
        mutable_statics(src.parse().expect("control tokenizes"), &mut found);
        found.len()
    };
    assert_eq!(
        scan("static FAILED: std::sync::atomic::AtomicBool = AtomicBool::new(false);"),
        1
    );
    assert_eq!(
        scan("fn f() { static SEEN: Mutex<Vec<u32>> = Mutex::new(Vec::new()); }"),
        1
    );
    assert_eq!(
        scan("static SLOTS: [AtomicU32; 4] = [const { AtomicU32::new(0) }; 4];"),
        1
    );
    assert_eq!(
        scan("thread_local! { static LAST: Cell<u32> = const { Cell::new(0) }; }"),
        1
    );
    assert_eq!(scan("static mut COUNT: u32 = 0;"), 1);
    assert_eq!(
        scan("pub(super) static NID_TABLE: &[(u32, &str, &str)] = &[];"),
        0
    );
    assert_eq!(scan("static CELL_NAMES: [&str; 2] = [\"a\", \"b\"];"), 0);
    assert_eq!(
        scan("fn f(x: &'static str) -> impl Fn() + 'static { move || {} }"),
        0
    );
    assert_eq!(
        scan(
            "// static A: AtomicU32 = AtomicU32::new(0);\nconst S: &str = \"static B: Mutex<u8>\";"
        ),
        0
    );
}

#[test]
fn the_include_scan_flags_a_call_and_not_a_name_that_merely_reads_include() {
    let scan = |src: &str| {
        let mut found = Vec::new();
        include_calls(src.parse().expect("control tokenizes"), &mut found);
        found
    };
    assert_eq!(scan("include!(\"table.rs\");"), ["include!(\"table.rs\")"]);
    assert_eq!(
        scan("fn f() { include!(concat!(\"a\", \"b\")); }").len(),
        1,
        "the scan reads the call, whatever builds its argument"
    );
    assert_eq!(
        scan("include!{\"table.rs\"}").len() + scan("include![\"table.rs\"];").len(),
        2,
        "a call is a call in each of the three delimiters"
    );
    assert_eq!(
        scan("std::include!(\"table.rs\");").len(),
        1,
        "a qualified call is a call"
    );
    assert!(
        scan("fn f(include: u32) { if include != (0) {} }").is_empty(),
        "the ! of a != is a punct like any other; the group that follows names the call"
    );
    assert!(
        scan("use a::include; fn include_rows() {} struct S { include: bool }").is_empty(),
        "an item named include is not a call"
    );
    assert!(
        scan("let _ = include_str!(\"x\");").is_empty(),
        "include_str! is the macro list's, not this scan's"
    );
    assert!(
        scan("// include!(\"x.rs\");\nconst S: &str = \"include!(\\\"y.rs\\\")\";").is_empty(),
        "a comment and a string literal are not calls"
    );
}

#[test]
fn every_member_sits_on_one_side_of_the_host_line() {
    let manifest = read(&workspace_root().join("Cargo.toml"));
    let members = members(&manifest);
    assert!(
        members.len() >= RUNTIME.len(),
        "gate went vacuous: {} member(s) parsed from Cargo.toml",
        members.len()
    );
    for member in &members {
        let runtime = RUNTIME.contains(&member.as_str());
        let host = HOST.contains(&member.as_str());
        assert!(
            runtime != host,
            "workspace member {member} is on {} side of the host line; list it in exactly one \
             of RUNTIME and HOST in this guard",
            if runtime { "both" } else { "neither" }
        );
    }
    for listed in RUNTIME.iter().chain(HOST) {
        assert!(
            members.iter().any(|m| m == listed),
            "{listed} is listed in this guard but is not a workspace member"
        );
    }
}

#[test]
fn every_runtime_crate_root_forbids_host_access() {
    let root = workspace_root();
    for member in RUNTIME {
        let lib = root.join(member).join("src").join("lib.rs");
        let parsed = syn::parse_file(&read(&lib))
            .unwrap_or_else(|e| panic!("cannot parse {}: {e}", lib.display()));
        assert!(
            forbids_host_access(&parsed),
            "{} lacks the crate-level attribute that makes the workspace clippy.toml's \
             disallowed-methods and disallowed-macros lists binding there:\n\
             #![cfg_attr(not(test), forbid(clippy::disallowed_methods, \
             clippy::disallowed_macros, clippy::print_stdout, clippy::print_stderr, \
             clippy::dbg_macro))]",
            lib.display()
        );
    }
}

#[test]
fn every_runtime_crate_has_no_crate_root_but_its_library() {
    let root = workspace_root();
    for member in RUNTIME {
        let other = other_crate_roots(&root.join(member));
        assert!(
            other.is_empty(),
            "{member} carries a crate root besides src/lib.rs, which no host-line \
             attribute covers: {other:?}"
        );
    }
}

/// Every non-test source of every runtime crate.
///
/// The floor applies per member. One floor over the whole set holds at
/// one file per crate, which is what a walk that stops early gives.
fn runtime_sources() -> Vec<PathBuf> {
    let root = workspace_root();
    let mut files = Vec::new();
    for member in RUNTIME {
        let before = files.len();
        non_test_sources(&root.join(member).join("src"), &mut files);
        assert!(
            files.len() > before,
            "gate went vacuous: {member} contributed no source file"
        );
    }
    files
}

#[test]
fn no_runtime_source_keeps_a_mutable_static() {
    let files = runtime_sources();
    let mut report = String::new();
    for file in &files {
        let stream: TokenStream = read(file)
            .parse()
            .unwrap_or_else(|e| panic!("cannot tokenize {}: {e}", file.display()));
        let mut found = Vec::new();
        mutable_statics(stream, &mut found);
        for decl in found {
            report.push_str(&format!("  {}: {decl}\n", file.display()));
        }
    }
    assert!(
        report.is_empty(),
        "runtime sources keep process-global state in a static; hold it in a value the \
         runtime owns instead:\n{report}"
    );
}

#[test]
fn no_runtime_source_reads_the_build_machine_with_include() {
    let files = runtime_sources();
    let mut report = String::new();
    for file in &files {
        let stream: TokenStream = read(file)
            .parse()
            .unwrap_or_else(|e| panic!("cannot tokenize {}: {e}", file.display()));
        let mut found = Vec::new();
        include_calls(stream, &mut found);
        for call in found {
            report.push_str(&format!("  {}: {call}\n", file.display()));
        }
    }
    assert!(
        report.is_empty(),
        "runtime sources read the build machine's file system; take the bytes from a \
         file source the host installs instead:\n{report}"
    );
}

#[test]
fn no_member_directory_shadows_the_workspace_clippy_toml() {
    let root = workspace_root();
    let members = members(&read(&root.join("Cargo.toml")));
    assert!(!members.is_empty(), "gate went vacuous: no member parsed");
    for member in &members {
        let mut dir = root.join(member);
        while dir != root {
            for name in ["clippy.toml", ".clippy.toml"] {
                let shadow = dir.join(name);
                assert!(
                    !shadow.exists(),
                    "{} replaces the workspace clippy.toml for {member}, lists included",
                    shadow.display()
                );
            }
            if !dir.pop() {
                break;
            }
        }
    }
}

#[test]
fn the_workspace_lists_keep_every_path_the_guard_requires() {
    let clippy_toml = read(&workspace_root().join("clippy.toml"));
    let methods = listed_paths(&clippy_toml, "disallowed-methods");
    for required in REQUIRED_METHODS {
        assert!(
            methods.iter().any(|m| m == required),
            "clippy.toml's disallowed-methods no longer names {required}"
        );
    }
    let macros = listed_paths(&clippy_toml, "disallowed-macros");
    for required in REQUIRED_MACROS {
        assert!(
            macros.iter().any(|m| m == required),
            "clippy.toml's disallowed-macros no longer names {required}"
        );
    }
    let types = listed_paths(&clippy_toml, "disallowed-types");
    for required in REQUIRED_TYPES {
        assert!(
            types.iter().any(|t| t == required),
            "clippy.toml's disallowed-types no longer names {required}"
        );
    }
}
