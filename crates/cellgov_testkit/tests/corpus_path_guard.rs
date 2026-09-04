//! Convention guards on which trees a source file may name.
//!
//! No source reads an RPCS3 install tree. CellGov owns its corpus:
//!
//! - `cellgov firmware install` writes firmware into `dev_flash`.
//! - `cellgov title install` writes titles.
//! - Committed data under `tests/fixtures/` holds what RPCS3 alone
//!   can answer.
//!
//! The RPCS3 *source* checkout is a different directory and stays
//! allowed.
//!
//! No test names a tree git does not track. A test that reads an
//! operator-owned or built tree cannot run on a fresh clone. The
//! corpus cargo features declare that dependency at the build level.
//! This guard covers the other half, where a path string reaches a
//! tree nobody else has. It reads the tree list from `.gitignore`, so
//! it covers a new ignored tree the day someone adds it.

use std::fs;
use std::path::{Path, PathBuf};

/// Assembled at run time so this file does not itself contain the
/// banned needle and can be scanned like every other source.
fn banned_prefix() -> String {
    ["tools", "rpcs3"].join("/")
}

/// Suffix that makes the RPCS3 source checkout a directory of its own
/// beside the install tree.
const SOURCE_CHECKOUT_SUFFIX: &str = "-src";

/// Fold a line to one spelling: ASCII-lowercase, backslash separators
/// to forward slashes, runs of separators to one, and `/./` to `/`.
///
/// Without it a Windows-style, uppercase, doubled-separator, or
/// dot-component spelling of the same tree slips past a plain
/// substring test.
fn normalize(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    for ch in line.chars() {
        let ch = if ch == '\\' {
            '/'
        } else {
            ch.to_ascii_lowercase()
        };
        if ch == '/' {
            if out.ends_with('/') {
                continue;
            }
            if out.ends_with("/.") {
                out.pop();
                continue;
            }
        }
        out.push(ch);
    }
    out
}

/// Whether `line` names the install tree, as a whole path component.
///
/// A trailing separator is not required -- a bare prefix reaches the
/// same tree through one `join` -- but the character after the prefix
/// must not be a name character, or every sibling directory that
/// merely starts with the same letters gets flagged too. The sibling
/// that matters is [`SOURCE_CHECKOUT_SUFFIX`].
fn names_install_tree(line: &str, needle: &str) -> bool {
    let normalized = normalize(line);
    normalized.match_indices(needle).any(|(at, _)| {
        normalized[at + needle.len()..]
            .chars()
            .next()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
    })
}

/// A directory tree `.gitignore` excludes.
struct IgnoredTree {
    name: String,
    /// A separator anywhere but the end of the rule pins it to the
    /// repo root. `/keys/` and `docs/dev/` each name one directory
    /// there; `vfs/.cellgov/keys` is a different tree that ends in
    /// the same component.
    anchored: bool,
}

/// Whether `line` names `tree`, at both edges.
///
/// Both edges must read as path context -- a separator, a quote, or
/// the end of the line -- and one edge must be a separator. The
/// leading edge rejects a sibling that ends the same way:
/// `tests/micro/.scratch/` is not the ignored `scratch`, and
/// `subkeys/` is not `keys`.
///
/// An anchored tree adds one condition: the match starts the path,
/// either at the opening quote or after the `../` run that a
/// crate-relative path walks back to the workspace root with.
fn names_tree(line: &str, tree: &IgnoredTree) -> bool {
    // A backtick marks rustdoc code and does not delimit a path: `a
    // `scratch` region` names a memory region, not the ignored tree.
    let path_context = |c: char| matches!(c, '/' | '"' | '\'');
    let normalized = normalize(line);
    normalized.match_indices(&tree.name).any(|(at, _)| {
        let before = normalized[..at].chars().next_back();
        let after = normalized[at + tree.name.len()..].chars().next();
        let edges = before.is_none_or(path_context) && after.is_none_or(path_context);
        // A path INTO the tree carries a separator on one side. Without
        // this, any quoted word equal to a tree name is a hit:
        // "target" is a thread name and "scratch" a memory region.
        let is_a_path = before == Some('/') || after == Some('/');
        // An anchored rule reaches only the repo root, and
        // `"../../keys/vault.toml"` reaches it as surely as
        // `"keys/vault.toml"` does. A rule that rejects every match
        // with a separator in front drops the spelling this workspace
        // uses to leave a crate directory.
        let rooted = !tree.anchored
            || normalized[..at]
                .trim_end_matches(['.', '/'])
                .chars()
                .next_back()
                .is_none_or(|c| c == '"' || c == '\'');
        edges && is_a_path && rooted
    })
}

/// Directory trees `.gitignore` excludes.
///
/// The parse skips glob lines and any entry whose last component
/// looks like a filename. That skip drops `tests/micro/**/build/`;
/// the microtest corpus features declare that tree instead. `vendor/`
/// and `third_party/` are not in this repo's `.gitignore` -- they are
/// the directories a contributor would use for a vendored dependency.
///
/// # Panics
///
/// Panics when `.gitignore` yields no tree, which leaves the caller
/// with the two conventional names alone.
fn ignored_trees(root: &Path) -> Vec<IgnoredTree> {
    let text = fs::read_to_string(root.join(".gitignore")).expect("the repo tracks a .gitignore");
    let mut trees: Vec<IgnoredTree> = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('!') {
            continue;
        }
        if line.contains('*') || line.contains('?') {
            continue;
        }
        let had_separator = line.ends_with('/');
        // A separator at the start or in the middle of a rule makes it
        // relative to the `.gitignore`'s own directory. Only a rule
        // whose sole separator is the trailing one matches at any
        // depth. `git check-ignore -v --no-index docs/dev/x sub/docs/dev/x`
        // names the `docs/dev/` rule for the first path and nothing for
        // the second.
        let anchored = line.starts_with('/') || line.trim_end_matches('/').contains('/');
        let entry = line.trim_start_matches('/').trim_end_matches('/');
        if entry.is_empty() {
            continue;
        }
        // `keys.toml` and `CLAUDE.local.md` are files. A dotted final
        // component with no trailing separator is one of those, while
        // `.venv/` and `.claude/` declare themselves with the slash.
        let dotted = entry.rsplit('/').next().is_some_and(|c| c.contains('.'));
        if dotted && !had_separator {
            continue;
        }
        trees.push(IgnoredTree {
            name: normalize(entry),
            anchored,
        });
    }
    assert!(
        !trees.is_empty(),
        "no directory tree parsed from {}",
        root.join(".gitignore").display()
    );
    for conventional in ["vendor", "third_party"] {
        trees.push(IgnoredTree {
            name: conventional.to_string(),
            anchored: false,
        });
    }
    trees.sort_by(|a, b| (&a.name, a.anchored).cmp(&(&b.name, b.anchored)));
    trees.dedup_by(|a, b| a.name == b.name && a.anchored == b.anchored);
    trees
}

/// Whether `path` is test code: a `.rs` file under a `tests/`
/// directory, or one named `*_tests.rs`.
///
/// `path` must be repo-relative. An absolute path reads the
/// checkout's own parent directories as workspace structure, so a
/// clone under a `tests` directory classifies every source file as
/// test code.
fn is_test_file(path: &Path) -> bool {
    let under_tests = path
        .components()
        .any(|c| c.as_os_str().to_string_lossy() == "tests");
    let named_tests = path
        .file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with("_tests.rs"));
    under_tests || named_tests
}

/// The guards whose own subject is the tree boundary.
///
/// Each guard names the trees it polices, so the rule it defines does
/// not apply to it.
fn defines_the_boundary(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with("_guard.rs"))
}

/// The corpus features, read from the CI workflow's `CORPUS_FEATURES`.
///
/// CI type-checks exactly that list, so this guard reads it there and
/// cannot drift from the workflow. Entries arrive as `crate/feature`;
/// only the feature half appears in a `cfg`.
fn corpus_features(root: &Path) -> Vec<String> {
    let ci = root.join(".github").join("workflows").join("ci.yml");
    let text =
        fs::read_to_string(&ci).unwrap_or_else(|e| panic!("cannot read {}: {e}", ci.display()));
    let after = text
        .split_once("CORPUS_FEATURES:")
        .map(|(_, rest)| rest)
        .expect("ci.yml declares CORPUS_FEATURES");
    let list: String = after
        .lines()
        .skip_while(|l| l.trim().is_empty() || l.trim() == ">-")
        .take_while(|l| l.contains('/'))
        .collect();
    let features: Vec<String> = list
        .split(',')
        .filter_map(|entry| entry.trim().split_once('/'))
        .map(|(_, feature)| feature.trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    assert!(
        !features.is_empty(),
        "no corpus features parsed from {}",
        ci.display()
    );
    features
}

/// Whether the file declares a corpus feature in a `cfg`.
///
/// A cargo feature is how a suite declares a local corpus, so that
/// declaration exempts the file from this guard.
fn declares_a_corpus_feature(source: &str, features: &[String]) -> bool {
    features
        .iter()
        .any(|f| source.contains(&format!("feature = \"{f}\"")))
}

/// Integration-test target names a `Cargo.toml` gates behind a corpus
/// feature via `required-features`.
///
/// The manifest gate and the in-source `cfg` say the same thing. A
/// target that uses one carries no trace of the other, so this guard
/// reads both. `apps/<crate>/tests/<name>.rs` is the target `<name>`.
fn manifest_gated_targets(manifest: &Path, features: &[String]) -> Vec<String> {
    let Ok(text) = fs::read_to_string(manifest) else {
        return Vec::new();
    };
    let mut gated = Vec::new();
    for block in text.split("[[test]]").skip(1) {
        let block = block.split("\n[").next().unwrap_or(block);
        let field = |key: &str| {
            block
                .lines()
                .find_map(|l| l.trim().strip_prefix(key)?.split_once('='))
                .map(|(_, v)| v.trim().to_string())
        };
        let (Some(name), Some(required)) = (field("name"), field("required-features")) else {
            continue;
        };
        if features.iter().any(|f| required.contains(f.as_str())) {
            gated.push(name.trim_matches('"').to_string());
        }
    }
    gated
}

/// Whether `file` is an integration-test target its own crate gates
/// behind a corpus feature.
fn manifest_gates(root: &Path, file: &Path, features: &[String]) -> bool {
    let Ok(relative) = file.strip_prefix(root) else {
        return false;
    };
    let parts: Vec<String> = relative
        .components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect();
    // apps/<crate>/tests/<name>.rs -- anything deeper is a helper
    // module, which carries no target name of its own.
    let parts: Vec<&str> = parts.iter().map(String::as_str).collect();
    let [group, krate, "tests", file_name] = parts.as_slice() else {
        return false;
    };
    let Some(target) = file_name.strip_suffix(".rs") else {
        return false;
    };
    let manifest = root.join(group).join(krate).join("Cargo.toml");
    manifest_gated_targets(&manifest, features)
        .iter()
        .any(|t| t == target)
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

/// Spellings are assembled, never written literally: a literal would
/// make this file a violation of the guard it defines.
#[test]
fn matcher_catches_every_spelling_of_the_install_tree() {
    let needle = banned_prefix();
    let install = ["tools", "rpcs3"].join("/");
    for spelling in [
        install.clone(),
        install.clone() + "/dev_hdd0/game",
        install.to_uppercase(),
        ["tools", "rpcs3", ""].join("\\"),
        ["tools", "", "rpcs3"].join("/"),
        ["tools", ".", "rpcs3"].join("/"),
    ] {
        assert!(
            names_install_tree(&format!("    let p = \"../../{spelling}\";"), &needle),
            "spelling {spelling:?} slipped past the matcher"
        );
    }
}

#[test]
fn matcher_leaves_the_source_checkout_alone() {
    let needle = banned_prefix();
    let checkout = ["tools", "rpcs3"].join("/") + SOURCE_CHECKOUT_SUFFIX;
    for spelling in [
        checkout.clone(),
        checkout.clone() + "/rpcs3/Emu/Cell",
        checkout.to_uppercase(),
        ["tools", "rpcs3-src", "build-msvc"].join("\\"),
        ["tools", "", "rpcs3-src"].join("/"),
    ] {
        assert!(
            !names_install_tree(&format!("//! see {spelling}"), &needle),
            "the source checkout {spelling:?} was flagged as an install tree"
        );
    }
}

#[test]
fn matcher_leaves_siblings_that_merely_share_the_prefix_alone() {
    let needle = banned_prefix();
    let base = ["tools", "rpcs3"].join("/");
    for tail in ["src", "-source", "_src", "-patch", "-"] {
        let spelling = base.clone() + tail;
        assert!(
            !names_install_tree(&format!("//! see {spelling}"), &needle),
            "{spelling:?} is not the install tree but was flagged"
        );
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

/// Every `.rs` file the guard is responsible for.
fn scanned_rs_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for group in ["crates", "apps", "bridges"] {
        rs_files_under(&root.join(group), &mut files);
    }
    files
}

/// The scan reaches nested `tests/` directories, so an empty result is
/// a broken walk rather than a clean workspace.
#[test]
fn the_scan_set_contains_this_guard() {
    let root = workspace_root();
    let files = scanned_rs_files(&root);
    let me = root
        .join("crates")
        .join("cellgov_testkit")
        .join("tests")
        .join("corpus_path_guard.rs");
    assert!(
        files.contains(&me),
        "the scan did not reach {}; {} files were collected",
        me.display(),
        files.len()
    );
}

#[test]
fn no_source_reads_an_rpcs3_install_tree() {
    let needle = banned_prefix();

    let files = scanned_rs_files(&workspace_root());
    assert!(
        !files.is_empty(),
        "no .rs files found under crates/apps/bridges"
    );

    let mut violations = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        for (n, line) in source.lines().enumerate() {
            if names_install_tree(line, &needle) {
                violations.push((file.clone(), n + 1));
            }
        }
    }
    violations.sort();

    let mut report = String::new();
    for (file, line) in &violations {
        report.push_str(&format!("  {}:{line}\n", file.display()));
    }
    assert!(
        violations.is_empty(),
        "sources naming an RPCS3 install tree. CellGov's corpus is \
         vfs/ (from the cellgov install commands) plus committed data under \
         tests/fixtures/; an RPCS3 install is one operator's machine \
         state, not a fixture location. The RPCS3 source checkout \
         (tools/rpcs3-src/) is allowed and unaffected:\n{report}"
    );
}

#[test]
fn gitignore_yields_the_trees_and_not_the_file_rules() {
    let trees = ignored_trees(&workspace_root());
    let names: Vec<&str> = trees.iter().map(|t| t.name.as_str()).collect();
    for expected in ["tools", "vfs", "scripts", "vendor", "docs/dev"] {
        assert!(
            names.contains(&expected),
            "{expected} is missing from the tree list: {names:?}"
        );
    }
    for glob_or_file in ["keys.toml", "*.htrc", "claude.local.md"] {
        assert!(
            !names.iter().any(|t| t.contains(glob_or_file)),
            "{glob_or_file} is a file rule and is not a tree"
        );
    }
    // `/keys/` carries the leading slash; `vfs/` does not.
    let anchored = |name: &str| {
        trees
            .iter()
            .find(|t| t.name == name)
            .is_some_and(|t| t.anchored)
    };
    assert!(anchored("keys"), "the /keys/ rule reads as repo-root only");
    assert!(!anchored("vfs"), "the vfs/ rule matches at any depth");
    assert!(
        anchored("docs/dev"),
        "an inner separator anchors a rule the way a leading one does"
    );
}

#[test]
fn the_tree_matcher_respects_component_boundaries() {
    let scratch = &IgnoredTree {
        name: "scratch".to_string(),
        anchored: false,
    };
    for named in ["\"scratch/a\"", "\"../../scratch\"", "\"/scratch\""] {
        assert!(
            names_tree(&format!("  let p = {named};"), scratch),
            "{named} names the tree but was missed"
        );
    }
    for sibling in [".scratch/", "sub-scratch/", "my_scratch/", "prescratch/"] {
        assert!(
            !names_tree(&format!("  let p = \"tests/{sibling}\";"), scratch),
            "{sibling} is not the ignored tree but was flagged"
        );
    }
    // Several trees are also ordinary words, which is why prose
    // context does not count as path context.
    for prose in [
        "// scratch that idea",
        "/// a scratch buffer",
        "  reject(&args, \"a manifest.toml scratch\")",
        "/// plus a `scratch` region with one more run",
        "            name: \"scratch\".into(),",
    ] {
        assert!(
            !names_tree(prose, scratch),
            "{prose:?} is prose, not a path"
        );
    }
}

#[test]
fn an_anchored_tree_matches_at_the_repo_root_and_nowhere_below_it() {
    let keys = &IgnoredTree {
        name: "keys".to_string(),
        anchored: true,
    };
    for rooted in [
        "\"keys/vault.toml\"",
        "join(\"keys/\")",
        "\"../../keys/vault.toml\"",
        "\"./keys/\"",
        "\"../keys\"",
    ] {
        assert!(
            names_tree(&format!("  let p = {rooted};"), keys),
            "{rooted} names the repo-root tree but was missed"
        );
    }
    for elsewhere in [
        "\".cellgov/keys\"",
        "\"d:/elsewhere/keys\"",
        "\"data/keys\"",
    ] {
        assert!(
            !names_tree(&format!("  let p = {elsewhere};"), keys),
            "{elsewhere} is a different directory that ends the same way"
        );
    }
}

#[test]
fn a_crate_relative_walk_to_the_root_still_names_an_anchored_tree() {
    let hello = &IgnoredTree {
        name: "tests/micro/hello_world".to_string(),
        anchored: true,
    };
    assert!(names_tree(
        "    let elf = \"../../tests/micro/hello_world/build/hello.elf\";",
        hello
    ));
    assert!(!names_tree(
        "    let elf = \"pkg/tests/micro/hello_world/build/hello.elf\";",
        hello
    ));
}

#[test]
fn ci_yields_the_corpus_features_the_exemption_reads() {
    let features = corpus_features(&workspace_root());
    for expected in ["firmware-corpus", "title-corpus", "microtests", "rpcs3-src"] {
        assert!(
            features.iter().any(|f| f == expected),
            "{expected} is missing from the corpus features: {features:?}"
        );
    }
    assert!(
        features.iter().all(|f| !f.contains('/')),
        "a feature name carries no crate half: {features:?}"
    );
    let gated = format!("#[cfg(feature = \"{}\")]", features[0]);
    assert!(declares_a_corpus_feature(&gated, &features));
    assert!(!declares_a_corpus_feature("#[cfg(test)]", &features));
}

#[test]
fn no_test_names_a_tree_git_does_not_track() {
    let root = workspace_root();
    let trees = ignored_trees(&root);

    let files: Vec<PathBuf> = scanned_rs_files(&root)
        .into_iter()
        .filter(|f| is_test_file(f.strip_prefix(&root).unwrap_or(f)) && !defines_the_boundary(f))
        .collect();
    assert!(
        !files.is_empty(),
        "no test files found under crates/apps/bridges"
    );

    let features = corpus_features(&root);
    let mut violations = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", file.display()));
        if declares_a_corpus_feature(&source, &features) || manifest_gates(&root, file, &features) {
            continue;
        }
        for (n, line) in source.lines().enumerate() {
            for tree in &trees {
                if names_tree(line, tree) {
                    let shown = file.strip_prefix(&root).unwrap_or(file);
                    violations.push((shown.display().to_string(), n + 1, tree.name.clone()));
                    break;
                }
            }
        }
    }
    violations.sort();

    let mut report = String::new();
    for (file, line, tree) in &violations {
        report.push_str(&format!("  {file}:{line}  names {tree}\n"));
    }
    assert!(
        violations.is_empty(),
        "{} test line(s) name a tree git does not track. A test that \
         reads an operator-owned or built tree cannot run on a fresh \
         clone: declare the dependency with a corpus feature, point the \
         test at committed data under tests/fixtures/, or delete it. \
         Files carrying a corpus-feature cfg, and the guards whose \
         subject is the boundary, are already exempt:\n{report}",
        violations.len()
    );
}
