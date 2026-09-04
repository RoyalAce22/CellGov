//! Convention guard: a fixture path a test names must be reachable.
//!
//! Two failure modes this catches.
//!
//! A test that names a gitignored file can only ever skip, and a skip
//! reads as a pass. Capture intermediates are the usual source: a run
//! writes one, the author points a test at it, and the test goes quiet
//! on every other machine forever.
//!
//! A test that names a committed fixture which does not exist fails on
//! a fresh clone for a reason the message rarely explains.
//!
//! Corpus fixtures are the exception. Those are operator-owned or
//! locally built, gitignored, and their suites sit behind a
//! cargo feature that hard-asserts when the file is absent. They are
//! listed in [`CORPUS_PREFIXES`].

use std::fs;
use std::path::{Path, PathBuf};

/// Path prefixes whose contents are gitignored and feature-gated.
///
/// `tests/micro/<name>/build/` holds ELFs produced by that test's
/// `build.sh` in a ps3dev toolchain container. The suites reading them
/// are behind `spu-microtests` / `ppu-microtests` / `microtests`.
const CORPUS_PREFIXES: &[&str] = &["tests/micro/", "tests/ps3autotests/"];

/// The subset of [`CORPUS_PREFIXES`] whose whole tree `.gitignore`
/// drops, so only an operator who cloned it has the directory at all.
/// The rest commit their sources and gitignore only build artifacts.
const OPERATOR_CORPUS_PREFIXES: &[&str] = &["tests/ps3autotests/"];

/// Extensions that only ever name a capture intermediate. A committed
/// artifact never carries one, so a test naming one is reading
/// something `.gitignore` drops.
const INTERMEDIATE_EXTENSIONS: &[&str] = &["tty", "dump", "htrc"];

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
        let kind = entry
            .file_type()
            .unwrap_or_else(|e| panic!("cannot stat {}: {e}", entry.path().display()));
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

fn scanned_rs_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for group in ["crates", "apps", "bridges"] {
        rs_files_under(&root.join(group), &mut files);
    }
    files
}

/// One `tests/...` literal and where it was written.
struct FixtureLiteral {
    /// The literal with any `../` crate-escape stripped.
    repo_path: String,
    /// True when the literal spelled its own escape out of the crate
    /// directory, which fixes the base it resolves against.
    anchored: bool,
    source: PathBuf,
    line: usize,
}

/// Every `tests/` path literal in the scanned sources.
///
/// Two spellings reach here. `"../../tests/..."` names its own base, so
/// [`FixtureLiteral::repo_path`] resolves against the workspace root.
/// A bare `"tests/..."` is joined onto a base the scan cannot see -- a
/// crate manifest dir, the workspace root, a temp dir -- so only the
/// base-independent extension rule may read it.
///
/// Literals holding a `{}` are `format!` templates whose value is only
/// known at run time; the directory they sit under is still checked
/// through the concrete literals beside them.
fn fixture_literals(root: &Path) -> Vec<FixtureLiteral> {
    let own_source = guard_source_path(root);
    let mut found = Vec::new();
    for file in scanned_rs_files(root) {
        // This file's positive controls are `tests/...` literals that
        // name nothing on disk; scanning them would make the guard
        // report itself.
        if file == own_source {
            continue;
        }
        let source = fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        for (n, line) in source.lines().enumerate() {
            for (repo_path, anchored) in literals_in_line(line) {
                found.push(FixtureLiteral {
                    repo_path,
                    anchored,
                    source: file.clone(),
                    line: n + 1,
                });
            }
        }
    }
    found
}

/// The `tests/` path literals on one source line, as
/// `(repo_path, anchored)`.
fn literals_in_line(line: &str) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let Some(end) = after.find('"') else {
            break;
        };
        let literal = &after[..end];
        rest = &after[end..];
        if literal.contains('{') {
            continue;
        }
        let stripped = literal.trim_start_matches("../");
        if !stripped.starts_with("tests/") {
            continue;
        }
        // `#[path = "tests/foo_tests.rs"]` names a module, not a fixture.
        if stripped.ends_with(".rs") {
            continue;
        }
        out.push((stripped.to_string(), stripped.len() != literal.len()));
    }
    out
}

fn guard_source_path(root: &Path) -> PathBuf {
    root.join("crates")
        .join("cellgov_testkit")
        .join("tests")
        .join("fixture_path_guard.rs")
}

fn is_corpus(repo_path: &str) -> bool {
    CORPUS_PREFIXES.iter().any(|p| repo_path.starts_with(p))
}

/// Whether `repo_path` names a capture intermediate by extension.
fn is_capture_intermediate(repo_path: &str) -> bool {
    let ext = Path::new(repo_path)
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    INTERMEDIATE_EXTENSIONS.contains(&ext.as_str())
}

/// Positive control: without it, an [`INTERMEDIATE_EXTENSIONS`] entry
/// respelled with a leading dot would make the gate below match nothing
/// and report green forever.
#[test]
fn the_intermediate_extension_matcher_flags_a_capture_and_spares_committed_data() {
    for path in [
        "tests/scenario_observations/flow/boot.tty",
        "tests/fixtures/NPUA80145/rpcs3/boot.dump",
        "tests/fixtures/NPUA80145/rpcs3/trace.HTRC",
    ] {
        assert!(is_capture_intermediate(path), "{path} should be flagged");
    }
    for path in [
        "tests/fixtures/NPUA80145/cellgov/anchors/fw-4.93/base/boot_summary.json",
        "tests/micro/process_spawn_wait/build/parent.elf",
        "tests/title_manifests/flow.toml",
        "tests/fixtures/NPUA80145",
    ] {
        assert!(
            !is_capture_intermediate(path),
            "{path} is committed data but was flagged"
        );
    }
}

/// Positive control for the corpus allowlist, whose only other use is
/// an early `continue` that a mis-spelled prefix would turn into a
/// silent no-op.
#[test]
fn the_corpus_allowlist_exempts_built_output_and_nothing_else() {
    assert!(is_corpus("tests/micro/process_spawn_wait/build/parent.elf"));
    assert!(is_corpus(
        "tests/ps3autotests/tests/cpu/basic/basic.ppu.elf"
    ));
    assert!(!is_corpus(
        "tests/fixtures/NPUA80145/cellgov/anchors/fw-4.93/base/boot_summary.json"
    ));
    assert!(!is_corpus("tests/title_manifests/flow.toml"));
}

/// Floors on the population each gate polices. The workspace has
/// carried well past these counts for several phases, so a collapse
/// below one of them is a broken matcher, not a tidied workspace.
const MIN_FIXTURE_LITERALS: usize = 40;
const MIN_ANCHORED_LITERALS: usize = 25;
const MIN_CHECKED_FIXTURES: usize = 6;

/// The one file the literal scan skips must still be reached by the
/// walk, or the skip is hiding a scan that never got there.
#[test]
fn the_scan_reaches_the_file_it_excludes_from_itself() {
    let root = workspace_root();
    let me = guard_source_path(&root);
    let files = scanned_rs_files(&root);
    assert!(
        files.contains(&me),
        "the scan did not reach {}; {} files were collected",
        me.display(),
        files.len()
    );
    assert!(
        !literals_in_line(r#"assert!(is_capture_intermediate("tests/x/boot.tty"));"#).is_empty(),
        "this file is skipped because it carries example tests/ literals;          if it no longer does, drop the skip"
    );
}

/// The scan reaches nested `tests/` directories, so an empty result is
/// a broken walk instead of a clean workspace.
#[test]
fn the_scan_finds_fixture_literals() {
    let found = fixture_literals(&workspace_root());
    assert!(
        found.len() >= MIN_FIXTURE_LITERALS,
        "gate went vacuous: only {} fixture literals found",
        found.len()
    );
    let anchored = found.iter().filter(|l| l.anchored).count();
    assert!(
        anchored >= MIN_ANCHORED_LITERALS,
        "gate went vacuous: only {anchored} of {} literals were anchored, so          the existence rule polices almost nothing",
        found.len()
    );
}

/// Positive control for the two spellings the scan must tell apart:
/// without it, dropping the `../` strip would silently retire the
/// existence rule.
#[test]
fn the_line_scanner_separates_anchored_literals_from_bare_ones() {
    assert_eq!(
        literals_in_line(r#"let p = base.join("../../tests/fixtures/x.json");"#),
        vec![("tests/fixtures/x.json".to_string(), true)]
    );
    assert_eq!(
        literals_in_line(r#"let p = root.join("tests/fixtures/x.json");"#),
        vec![("tests/fixtures/x.json".to_string(), false)]
    );
    assert_eq!(
        literals_in_line(r#"let p = root.join("tests").join("micro");"#),
        Vec::new(),
        "a split .join() chain carries no tests/ literal and stays invisible"
    );
    assert_eq!(
        literals_in_line(r#"let p = format!("tests/fixtures/{id}/boot.json");"#),
        Vec::new(),
        "a template's value is only known at run time"
    );
    assert_eq!(
        literals_in_line(r##"let p = "attests/x.json";"##),
        Vec::new(),
        "the needle is a path segment, not a suffix"
    );
    assert_eq!(
        literals_in_line(r##"#[path = "tests/host_tests.rs"]"##),
        Vec::new(),
        "a module path attribute names no fixture"
    );
}

#[test]
fn no_test_names_a_capture_intermediate() {
    let mut violations = Vec::new();
    for lit in fixture_literals(&workspace_root()) {
        if is_capture_intermediate(&lit.repo_path) {
            violations.push(format!(
                "  {}:{} -> {}",
                lit.source.display(),
                lit.line,
                lit.repo_path
            ));
        }
    }
    assert!(
        violations.is_empty(),
        "tests naming a capture intermediate. `.gitignore` drops these, so \
         the test can only skip, and a skip is indistinguishable from a \
         pass. Write the bytes the test wants into a scratch dir and parse \
         those back:\n{}",
        violations.join("\n")
    );
}

#[test]
fn every_committed_fixture_a_test_names_exists() {
    let root = workspace_root();
    let mut missing = Vec::new();
    let mut checked = 0usize;
    for lit in fixture_literals(&root) {
        // A bare literal is joined onto a base the scan cannot see, so
        // the workspace root is not where it resolves.
        if !lit.anchored || is_corpus(&lit.repo_path) {
            continue;
        }
        checked += 1;
        if !root.join(&lit.repo_path).exists() {
            missing.push(format!(
                "  {}:{} -> {}",
                lit.source.display(),
                lit.line,
                lit.repo_path
            ));
        }
    }
    assert!(
        checked >= MIN_CHECKED_FIXTURES,
        "gate went vacuous: only {checked} literal(s) reached the existence          check, expected at least {MIN_CHECKED_FIXTURES}. Either the corpus          allowlist swallowed the population or the scan lost its anchored          literals"
    );
    assert!(
        missing.is_empty(),
        "tests naming committed fixtures that do not exist. A path outside \
         {CORPUS_PREFIXES:?} is committed data and must be present in a \
         fresh clone:\n{}",
        missing.join("\n")
    );
}

#[test]
fn corpus_prefixes_name_directories_that_exist() {
    let root = workspace_root();
    for prefix in CORPUS_PREFIXES
        .iter()
        .filter(|p| !OPERATOR_CORPUS_PREFIXES.contains(p))
    {
        let dir = root.join(prefix.trim_end_matches('/'));
        assert!(
            dir.is_dir(),
            "corpus prefix {prefix} names {}, which is not a directory; the \
             allowlist would silently exempt nothing",
            dir.display()
        );
    }
}

#[test]
fn operator_corpus_prefixes_are_dropped_by_name_in_gitignore() {
    let root = workspace_root();
    let ignore = fs::read_to_string(root.join(".gitignore")).expect("workspace .gitignore");
    for prefix in OPERATOR_CORPUS_PREFIXES {
        let bare = prefix.trim_end_matches('/');
        assert!(
            CORPUS_PREFIXES.contains(prefix),
            "{prefix} is absent from the corpus allowlist, so it exempts nothing"
        );
        assert!(
            ignore.lines().any(|line| {
                let line = line.trim();
                !line.starts_with('#') && line.trim_start_matches('/').trim_end_matches('/') == bare
            }),
            "operator corpus prefix {prefix} has no `.gitignore` rule naming \
             it. Either the tree is committed now, and the prefix belongs in \
             the existence check instead, or the prefix is stale and exempts \
             nothing"
        );
    }
}
