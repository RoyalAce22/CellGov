//! Convention guard: no CellGov crate reaches into the GPL subtree.
//!
//! `bridges/rpcs3-patch/` is GPL-2.0-only as modifications to RPCS3;
//! the rest of the workspace is Apache-2.0 / MIT. The two stay
//! separable only while nothing in a `cellgov_*` crate compiles,
//! links, or embeds anything from that directory -- RPCS3 is reached
//! by spawning `rpcs3.exe` and reading the files it writes.
//!
//! The violation is a build-time reach -- a manifest entry, a
//! `build.rs`, or an `include!` macro pulling the subtree's bytes into
//! a compiled artifact. Prose that merely names the directory passes.

use std::fs;
use std::path::{Path, PathBuf};

/// Assembled at run time so this file does not itself contain the
/// needle it scans for.
fn subtree_needle() -> String {
    ["rpcs3", "patch"].join("-")
}

/// The `include!` family: every macro that pulls a file's bytes into
/// the crate being compiled. No name here is a substring of another,
/// so one occurrence is counted once.
const INCLUDE_MACROS: [&str; 3] = ["include_str!", "include_bytes!", "include!"];

/// Floors on the populations the guard polices. A scan that collapses
/// below either found nothing because it walked nothing, and would
/// otherwise report the same empty violation list as a clean tree.
const MIN_MANIFESTS: usize = 10;
const MIN_SOURCES: usize = 100;

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

/// Fold to one spelling: ASCII-lowercase, backslashes to forward
/// slashes. Without it a Windows-style or capitalised path slips past
/// a plain substring test. Both transforms are 1:1 on bytes, so
/// offsets into the result still index the original's lines.
fn normalize(text: &str) -> String {
    text.replace('\\', "/").to_ascii_lowercase()
}

/// 1-based lines carrying an `include!`-family macro whose argument
/// names the subtree.
///
/// Scans the whole source: rustfmt wraps a long `include_str!` across
/// lines, and a per-line test then sees neither the macro nor its path
/// as a violation.
fn include_reaches(source: &str, needle: &str) -> Vec<usize> {
    let normalized = normalize(source);
    let mut hits = Vec::new();
    for macro_name in INCLUDE_MACROS {
        let mut from = 0;
        while let Some(rel) = normalized[from..].find(macro_name) {
            let at = from + rel;
            let arg_end = normalized[at..]
                .find(')')
                .map_or(normalized.len(), |end| at + end);
            if normalized[at..arg_end].contains(needle) {
                hits.push(normalized[..at].matches('\n').count() + 1);
            }
            from = at + macro_name.len();
        }
    }
    hits.sort_unstable();
    hits.dedup();
    hits
}

/// Every IO error panics with its path: a swallowed one shrinks the
/// set the guard inspects, and a scan that walked nothing reports the
/// same empty violation list as a clean tree.
fn files_under(dir: &Path, want: &dyn Fn(&Path) -> bool, out: &mut Vec<PathBuf>) {
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
            files_under(&path, want, out);
        } else if want(&path) {
            out.push(path);
        }
    }
}

fn read(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

fn is_manifest_or_build_script(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n == "Cargo.toml" || n == "build.rs")
}

/// Every manifest and build script the workspace compiles through.
fn manifests_and_build_scripts(root: &Path) -> Vec<PathBuf> {
    let mut found = vec![root.join("Cargo.toml")];
    for group in ["crates", "apps", "bridges"] {
        files_under(&root.join(group), &is_manifest_or_build_script, &mut found);
    }
    found
}

fn rust_sources(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let want = |p: &Path| p.extension().is_some_and(|e| e == "rs");
    for group in ["crates", "apps", "bridges"] {
        files_under(&root.join(group), &want, &mut found);
    }
    found
}

fn gpl_subtree(root: &Path) -> PathBuf {
    root.join("bridges").join(subtree_needle())
}

#[test]
fn the_matcher_separates_a_build_time_reach_from_a_prose_mention() {
    let needle = subtree_needle();
    let subtree = ["bridges", &subtree_needle()].join("/");

    for reach in [
        format!("const C: &str = include_str!(\"../../{subtree}/some.yml\");"),
        format!("include_bytes!(\"../../../{subtree}/blob.bin\")"),
        format!("include!(\"..\\\\..\\\\{subtree}\\\\generated.rs\");"),
        // rustfmt wraps a long one; the macro and the path split.
        format!("const C: &str = include_str!(\n    \"../../{subtree}/some.yml\"\n);"),
    ] {
        assert!(
            !include_reaches(&reach, &needle).is_empty(),
            "a build-time reach slipped past the matcher: {reach}"
        );
    }

    for allowed in [
        format!("//! the patched build ({subtree}/0002-cellgov-hle-trace.patch)"),
        format!("// see {subtree}/README.md for the env-var contract"),
        "const C: &str = include_str!(\"../oracle_mode_config.yml\");".to_string(),
    ] {
        assert!(
            include_reaches(&allowed, &needle).is_empty(),
            "a legitimate line was flagged as a reach: {allowed}"
        );
    }
}

#[test]
fn the_matcher_reports_the_line_the_macro_starts_on() {
    let needle = subtree_needle();
    let source = format!(
        "fn main() {{}}\n// filler\nconst C: &str = include_str!(\"../../bridges/{}/x.yml\");\n",
        subtree_needle()
    );
    assert_eq!(include_reaches(&source, &needle), vec![3]);
}

#[test]
fn the_gpl_subtree_carries_no_crate() {
    let subtree = gpl_subtree(&workspace_root());
    assert!(
        subtree.is_dir(),
        "expected the GPL subtree at {}",
        subtree.display()
    );

    let mut found = Vec::new();
    files_under(&subtree, &is_manifest_or_build_script, &mut found);
    assert!(
        found.is_empty(),
        "a Cargo.toml or build.rs appeared in the GPL subtree; it would \
         make the directory buildable as part of the Apache/MIT \
         workspace: {found:?}"
    );
}

#[test]
fn the_gpl_subtree_carries_its_own_license() {
    let copying = gpl_subtree(&workspace_root()).join("COPYING");
    assert!(
        copying.is_file(),
        "the GPL subtree must ship its license text at {}",
        copying.display()
    );
    let text = read(&copying);
    assert!(
        text.contains("GNU GENERAL PUBLIC LICENSE") && text.contains("Version 2, June 1991"),
        "{} is not the GPLv2 text",
        copying.display()
    );
}

#[test]
fn no_manifest_or_build_script_references_the_gpl_subtree() {
    let needle = subtree_needle();
    let files = manifests_and_build_scripts(&workspace_root());
    assert!(
        files.len() >= MIN_MANIFESTS,
        "gate went vacuous: only {} manifest(s) / build script(s) found, \
         expected at least {MIN_MANIFESTS}",
        files.len()
    );

    let mut violations = Vec::new();
    for file in &files {
        for (n, line) in read(file).lines().enumerate() {
            if normalize(line).contains(&needle) {
                violations.push(format!("  {}:{}\n", file.display(), n + 1));
            }
        }
    }
    violations.sort();
    assert!(
        violations.is_empty(),
        "a manifest or build script names the GPL subtree. Nothing in \
         bridges/rpcs3-patch/ may take part in the build; RPCS3 is \
         reached by spawning it, never by compiling against it:\n{}",
        violations.concat()
    );
}

#[test]
fn no_source_includes_a_file_from_the_gpl_subtree() {
    let needle = subtree_needle();
    let files = rust_sources(&workspace_root());
    assert!(
        files.len() >= MIN_SOURCES,
        "gate went vacuous: only {} source file(s) found, expected at \
         least {MIN_SOURCES}",
        files.len()
    );

    let mut violations = Vec::new();
    for file in &files {
        for line in include_reaches(&read(file), &needle) {
            violations.push(format!("  {}:{line}\n", file.display()));
        }
    }
    violations.sort();
    assert!(
        violations.is_empty(),
        "a source file embeds bytes from the GPL subtree. Move the file \
         it wants beside its consumer instead -- an include! puts the \
         subtree's terms on the compiled artifact:\n{}",
        violations.concat()
    );
}

/// A shipped patched binary would trigger GPLv2's section-3 source
/// duty that a source-only patch does not.
#[test]
fn the_rpcs3_tool_tree_stays_ignored() {
    let gitignore = workspace_root().join(".gitignore");
    let ignores_tools = read(&gitignore)
        .lines()
        .map(str::trim)
        .any(|line| matches!(line, "tools/" | "/tools/" | "tools" | "/tools"));

    assert!(
        ignores_tools,
        "{} no longer ignores tools/. The RPCS3 checkout and any built \
         rpcs3.exe live there; committing a patched binary is a \
         distribution this repo is not set up to make.",
        gitignore.display()
    );
}
