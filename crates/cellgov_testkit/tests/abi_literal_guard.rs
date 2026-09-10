//! Convention guard: PS3 ABI facts recognisable by shape live in
//! `cellgov_ps3_abi`.
//!
//! Two shapes need no reader to classify them. A `0x8001_xxxx` literal
//! is a Cell error code, and an integer arm in a syscall-number match
//! or a routed-syscall table is a syscall number. Both are values the
//! PS3 defines, so both belong in the ABI crate under a name, and a
//! copy in the crate that first needed one is where a second copy
//! starts.

use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, TokenStream, TokenTree};

/// The crate that owns the ABI vocabulary.
const ABI_CRATE: &str = "cellgov_ps3_abi";

/// Cell error codes share this prefix; nothing else in the guest ABI does.
const CELL_ERROR_PREFIX: &str = "0x8001";

/// Files whose integer match arms and table entries are syscall
/// numbers: the request classifier and the routed-syscall fidelity
/// table.
const SYSCALL_NUMBER_FILES: &[&str] = &[
    "crates/cellgov_lv2/src/request/classify.rs",
    "crates/cellgov_lv2/src/request/fidelity.rs",
];

/// Identifiers a syscall-number match scrutinises.
const SYSCALL_NUMBER_NAMES: &[&str] = &["syscall_num", "number", "num", "r11"];

/// Floors on the populations the guard polices.
const MIN_SOURCES: usize = 200;
const MIN_SYSCALL_ARMS: usize = 80;

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

/// Whether `path` is test code: under a `tests/` directory or named
/// `*_tests.rs`. `path` must be repo-relative.
fn is_test_file(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str().to_string_lossy() == "tests")
        || path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with("_tests.rs"))
}

/// Shipped, non-test Rust outside the ABI crate.
fn sources_outside_abi(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for group in ["crates", "apps", "bridges"] {
        rs_files_under(&root.join(group), &mut found);
    }
    found.retain(|f| {
        let rel = f.strip_prefix(root).unwrap_or(f);
        !is_test_file(rel)
            && !rel
                .components()
                .any(|c| c.as_os_str().to_string_lossy() == ABI_CRATE)
    });
    found.sort();
    found
}

fn tokens_of(source: &str) -> TokenStream {
    source.parse().expect("source tokenizes")
}

/// Whether a literal token spells an integer.
fn is_integer(lit: &proc_macro2::Literal) -> bool {
    lit.to_string().starts_with(|c: char| c.is_ascii_digit())
}

/// Lines carrying a Cell error-code literal outside a comment or a
/// string: the token stream carries neither.
fn cell_error_literals(source: &str) -> Vec<usize> {
    fn walk(stream: TokenStream, out: &mut Vec<usize>) {
        for tt in stream {
            match tt {
                TokenTree::Literal(lit) => {
                    let text = lit.to_string().replace('_', "").to_ascii_lowercase();
                    if text.starts_with(CELL_ERROR_PREFIX)
                        && text.len() == CELL_ERROR_PREFIX.len() + 4
                    {
                        out.push(lit.span().start().line);
                    }
                }
                TokenTree::Group(g) => walk(g.stream(), out),
                _ => {}
            }
        }
    }
    let mut out = Vec::new();
    walk(tokens_of(source), &mut out);
    out.sort_unstable();
    out.dedup();
    out
}

/// Tally and violations from the syscall-number shapes in one file.
#[derive(Default, Debug, PartialEq, Eq)]
struct SyscallScan {
    /// Match arms and table entries inspected.
    arms: usize,
    /// Lines whose arm or entry is a bare integer.
    literal_lines: Vec<usize>,
}

/// Inside a brace group that is a match body: count arms and flag an
/// arm whose pattern opens with an integer literal.
fn scan_match_body(body: TokenStream, scan: &mut SyscallScan) {
    let tokens: Vec<TokenTree> = body.into_iter().collect();
    let mut at_arm_start = true;
    let mut i = 0;
    while i < tokens.len() {
        match &tokens[i] {
            TokenTree::Punct(p) if p.as_char() == ',' => at_arm_start = true,
            TokenTree::Punct(p) if p.as_char() == '=' => {
                if matches!(tokens.get(i + 1), Some(TokenTree::Punct(q)) if q.as_char() == '>') {
                    scan.arms += 1;
                    i += 1;
                }
            }
            TokenTree::Literal(lit) if at_arm_start && is_integer(lit) => {
                scan.literal_lines.push(lit.span().start().line);
                at_arm_start = false;
            }
            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
                // An arm body; a comma may be absent after a block arm.
                at_arm_start = true;
                let _ = g;
            }
            _ => at_arm_start = false,
        }
        i += 1;
    }
}

/// Inside a bracket group that is a table: count tuple entries and flag
/// one whose first element is an integer literal.
fn scan_table(body: TokenStream, scan: &mut SyscallScan) {
    for tt in body {
        if let TokenTree::Group(g) = tt {
            if g.delimiter() == Delimiter::Parenthesis {
                scan.arms += 1;
                if let Some(TokenTree::Literal(lit)) = g.stream().into_iter().next() {
                    if is_integer(&lit) {
                        scan.literal_lines.push(lit.span().start().line);
                    }
                }
            }
        }
    }
}

fn scan_syscall_numbers(source: &str) -> SyscallScan {
    fn walk(stream: TokenStream, scan: &mut SyscallScan) {
        let tokens: Vec<TokenTree> = stream.into_iter().collect();
        let mut i = 0;
        while i < tokens.len() {
            match &tokens[i] {
                TokenTree::Ident(id) if id == "match" => {
                    // Scrutinee tokens run to the brace group.
                    let mut j = i + 1;
                    let mut names_syscall = false;
                    while j < tokens.len() {
                        match &tokens[j] {
                            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => break,
                            TokenTree::Ident(id) => {
                                names_syscall |=
                                    SYSCALL_NUMBER_NAMES.contains(&id.to_string().as_str());
                            }
                            _ => {}
                        }
                        j += 1;
                    }
                    if let Some(TokenTree::Group(g)) = tokens.get(j) {
                        if names_syscall {
                            scan_match_body(g.stream(), scan);
                        }
                        walk(g.stream(), scan);
                    }
                    i = j + 1;
                    continue;
                }
                TokenTree::Ident(id) if id == "ROUTED_UNSUPPORTED_ARMS" => {
                    // `: &[(u64, ...)] = { use ...; &[ ... ] }` or `= &[ ... ]`:
                    // the type annotation carries a bracket group too, so
                    // the table is the first bracket group after the `=`.
                    let mut j = i + 1;
                    while j < tokens.len()
                        && !matches!(&tokens[j], TokenTree::Punct(p) if p.as_char() == '=')
                    {
                        j += 1;
                    }
                    while j < tokens.len() {
                        match &tokens[j] {
                            TokenTree::Group(g) if g.delimiter() == Delimiter::Bracket => {
                                scan_table(g.stream(), scan);
                                break;
                            }
                            TokenTree::Group(g) if g.delimiter() == Delimiter::Brace => {
                                for inner in g.stream() {
                                    if let TokenTree::Group(b) = inner {
                                        if b.delimiter() == Delimiter::Bracket {
                                            scan_table(b.stream(), scan);
                                        }
                                    }
                                }
                                break;
                            }
                            TokenTree::Punct(p) if p.as_char() == ';' => break,
                            _ => {}
                        }
                        j += 1;
                    }
                    i = j + 1;
                    continue;
                }
                TokenTree::Group(g) => walk(g.stream(), scan),
                _ => {}
            }
            i += 1;
        }
    }
    let mut scan = SyscallScan::default();
    walk(tokens_of(source), &mut scan);
    scan.literal_lines.sort_unstable();
    scan.literal_lines.dedup();
    scan
}

#[test]
fn the_error_code_matcher_reads_literals_not_comments_or_strings() {
    let source = "// 0x8001_0002 in a comment\n\
                  const S: &str = \"0x80010002\";\n\
                  fn f() -> u64 { 0x8001_051D }\n\
                  fn g() -> u32 { 0x80010003 }\n\
                  fn h() -> u64 { 0x8001_0000_0000 }\n\
                  fn i() -> u32 { 0x1000_8001 }\n";
    assert_eq!(cell_error_literals(source), vec![3, 4]);
}

#[test]
fn the_syscall_matcher_counts_arms_and_flags_bare_numbers() {
    let scan = scan_syscall_numbers(
        "fn c(syscall_num: u64) -> R {\n\
             match syscall_num {\n\
                 syscall::PROCESS_GETPID => R::A,\n\
                 syscall::A | syscall::B => R::B,\n\
                 871 => R::C,\n\
                 12 | 13 => R::D,\n\
                 _ => R::E,\n\
             }\n\
         }\n\
         fn other(pkg_id: u64) -> u8 { match pkg_id { 1 | 3 => 0, _ => 1 } }\n",
    );
    assert_eq!(
        scan,
        SyscallScan {
            arms: 5,
            literal_lines: vec![5, 6],
        }
    );

    let scan = scan_syscall_numbers(
        "pub const ROUTED_UNSUPPORTED_ARMS: &[(u64, &str, F)] = {\n\
             use x::syscall;\n\
             &[(syscall::A, \"a\", F::Null), (44, \"b\", F::Null)]\n\
         };\n",
    );
    assert_eq!(
        scan,
        SyscallScan {
            arms: 2,
            literal_lines: vec![3],
        }
    );
}

#[test]
fn cell_error_codes_are_named_in_the_abi_crate() {
    let root = workspace_root();
    let files = sources_outside_abi(&root);
    assert!(
        files.len() >= MIN_SOURCES,
        "gate went vacuous: only {} source file(s) outside {ABI_CRATE}",
        files.len()
    );
    let mut violations = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        for line in cell_error_literals(&source) {
            let shown = file.strip_prefix(&root).unwrap_or(file);
            violations.push(format!("  {}:{line}\n", shown.display()));
        }
    }
    violations.sort();
    assert!(
        violations.is_empty(),
        "{} Cell error-code literal(s) outside {ABI_CRATE}. Name the code in \
         cellgov_ps3_abi::lv2::errno (or the subsystem module that owns the domain) and \
         refer to it:\n{}",
        violations.len(),
        violations.concat()
    );
}

#[test]
fn syscall_numbers_reach_the_classifier_and_the_tables_by_name() {
    let root = workspace_root();
    let mut arms = 0usize;
    let mut violations = Vec::new();
    for rel in SYSCALL_NUMBER_FILES {
        let path = root.join(rel);
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
        let scan = scan_syscall_numbers(&source);
        arms += scan.arms;
        for line in scan.literal_lines {
            violations.push(format!("  {rel}:{line}\n"));
        }
    }
    assert!(
        arms >= MIN_SYSCALL_ARMS,
        "gate went vacuous: only {arms} syscall arm(s) or table entr(ies) recognised"
    );
    violations.sort();
    assert!(
        violations.is_empty(),
        "{} syscall number(s) appear as bare integers. Declare the number in \
         cellgov_ps3_abi::lv2::syscall and match on the name:\n{}",
        violations.len(),
        violations.concat()
    );
}
