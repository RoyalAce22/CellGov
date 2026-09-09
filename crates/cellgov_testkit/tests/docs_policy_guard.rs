//! Convention guards on what the public text may name.
//!
//! - The architecture and concepts documents describe mechanism. No
//!   content id and no display name from the title registry appears in
//!   them; per-title state lives in `docs/titles.md`.
//! - Shipped Rust comments carry no issue-tracker reference. A `#N`
//!   in a comment means nothing to a reader without the tracker.
//! - No tracked text file points a reader at a path git does not
//!   track. The trees below are the ones a clone cannot populate:
//!   nothing describes how, so a pointer into one dangles for every
//!   public reader.

use std::fs;
use std::path::{Path, PathBuf};

/// Gitignored trees and files no README tells a reader how to obtain.
/// Each must still appear in `.gitignore`; the guard checks that, so
/// this list cannot outlive the rules it mirrors.
const UNTRACKED_TREES: &[&str] = &["docs/dev/", ".claude/", "scripts/", "CLAUDE.local.md"];

/// Extensions the dangling-pointer scan reads. JSON is generated data
/// in this tree, so the scan skips it.
const TEXT_EXTENSIONS: &[&str] = &["rs", "md", "toml", "txt", "yml", "yaml", "template"];

/// Floors on the populations each check polices. A collapse below one
/// means the scanner walked or parsed nothing, and would otherwise
/// report the same empty violation list as a conforming tree.
const MIN_IDENTITIES: usize = 4;
const MIN_MECHANISM_DOCS: usize = 10;
const MIN_COMMENT_LINES: usize = 20_000;
const MIN_TEXT_FILES: usize = 400;

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

fn has_extension(path: &Path, ext: &str) -> bool {
    path.extension().is_some_and(|e| e == ext)
}

/// Every `.md` under `docs/architecture/` and `docs/concepts/`.
fn mechanism_docs(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for dir in ["architecture", "concepts"] {
        files_under(
            &root.join("docs").join(dir),
            &|p| has_extension(p, "md"),
            &mut found,
        );
    }
    found
}

/// Shipped Rust under the three crate groups.
fn shipped_rust(root: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    for group in ["crates", "apps", "bridges"] {
        files_under(&root.join(group), &|p| has_extension(p, "rs"), &mut found);
    }
    found
}

/// The value of a `key = "..."` line in a TOML table, if the line is one.
fn toml_string(line: &str, key: &str) -> Option<String> {
    let rest = line
        .trim()
        .strip_prefix(key)?
        .trim_start()
        .strip_prefix('=')?
        .trim();
    let inner = rest.strip_prefix('"')?;
    let end = inner.find('"')?;
    Some(inner[..end].to_string())
}

/// The identity strings one manifest's `[title]` table declares.
///
/// A firmware-exec manifest names a system-software component, and
/// its content id is that component's name (`VSH`), which the
/// mechanism documents use as vocabulary. Only its display name
/// counts as identity.
fn manifest_identities(text: &str) -> Vec<String> {
    let mut content_id = None;
    let mut display_name = None;
    let mut firmware_exec = false;
    for line in text.lines() {
        if line.trim_start().starts_with('[') && line.trim() != "[title]" {
            if content_id.is_some() || display_name.is_some() {
                break;
            }
            continue;
        }
        content_id = content_id.or_else(|| toml_string(line, "content_id"));
        display_name = display_name.or_else(|| toml_string(line, "display_name"));
        firmware_exec |= toml_string(line, "distribution").is_some_and(|d| d == "firmware-exec");
    }
    let mut out = Vec::new();
    if let Some(id) = content_id.filter(|_| !firmware_exec) {
        out.push(id);
    }
    out.extend(display_name);
    out
}

/// Every identity string in the title registry. The template README is
/// prose, not a manifest, and is skipped.
fn title_identities(root: &Path) -> Vec<String> {
    let dir = root.join("title_manifests");
    let mut found = Vec::new();
    let mut manifests = Vec::new();
    files_under(&dir, &|p| has_extension(p, "toml"), &mut manifests);
    manifests.sort();
    for manifest in manifests {
        let identities = manifest_identities(&read(&manifest));
        assert!(
            !identities.is_empty(),
            "{} declares no [title] display_name",
            manifest.display()
        );
        found.extend(identities);
    }
    found
}

/// Whether `line` names the identity as a whole word. A content id is
/// one word; a display name is a phrase, and a phrase match is a
/// whole-word match of its ends.
fn names_identity(line: &str, identity: &str) -> bool {
    let word_char = |c: char| c.is_ascii_alphanumeric() || c == '_';
    line.match_indices(identity).any(|(at, _)| {
        let before = line[..at].chars().next_back();
        let after = line[at + identity.len()..].chars().next();
        !before.is_some_and(word_char) && !after.is_some_and(word_char)
    })
}

/// The comment body of a Rust source line, if the line is one.
fn comment_body(line: &str) -> Option<&str> {
    let t = line.trim_start();
    t.strip_prefix("//!")
        .or_else(|| t.strip_prefix("///"))
        .or_else(|| t.strip_prefix("//"))
}

/// Whether a comment carries a tracker reference: `#` and two or more
/// digits, standing alone. `#[attr]`, `#0x1234`, `mod#3`, and a `#1`
/// ordinal do not count.
fn has_issue_reference(comment: &str) -> bool {
    let bytes = comment.as_bytes();
    let mut i = 0;
    while let Some(rel) = comment[i..].find('#') {
        let at = i + rel;
        let before = comment[..at].chars().next_back();
        let digits = comment[at + 1..]
            .bytes()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let after = bytes.get(at + 1 + digits).copied();
        let preceded_by_word = before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_');
        let followed_by_word = after.is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_');
        if digits >= 2 && !preceded_by_word && !followed_by_word {
            return true;
        }
        i = at + 1;
    }
    false
}

/// Whether a line cites one of the untracked trees. The match must
/// start at a path boundary so `docs/dev/` does not fire on
/// `mydocs/dev/`.
fn cites_untracked_tree(line: &str) -> Option<&'static str> {
    UNTRACKED_TREES.iter().copied().find(|tree| {
        line.match_indices(tree).any(|(at, _)| {
            let before = line[..at].chars().next_back();
            !before.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.')
        })
    })
}

/// The guards whose own subject is a rule over source text. Each one
/// spells out the shapes it rejects, so the rule it defines does not
/// apply to it.
fn defines_a_rule(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with("_guard.rs"))
}

/// Tracked text files: the crate groups, the public docs, the fixtures,
/// the registry, the workflow definitions and the root documents.
fn tracked_text_files(root: &Path) -> Vec<PathBuf> {
    let text = |p: &Path| TEXT_EXTENSIONS.iter().any(|e| has_extension(p, e)) && !defines_a_rule(p);
    let mut found = Vec::new();
    for dir in [
        "crates",
        "apps",
        "bridges",
        "tests",
        "title_manifests",
        ".github",
    ] {
        files_under(&root.join(dir), &text, &mut found);
    }
    let docs = root.join("docs");
    let entries =
        fs::read_dir(&docs).unwrap_or_else(|e| panic!("cannot read {}: {e}", docs.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("cannot read entry under {}: {e}", docs.display()))
            .path();
        if path.is_dir() {
            if path.file_name().is_some_and(|n| n == "dev") {
                continue;
            }
            files_under(&path, &text, &mut found);
        } else if text(&path) {
            found.push(path);
        }
    }
    for name in ["README.md", "ACKNOWLEDGEMENTS.md", "Cargo.toml"] {
        found.push(root.join(name));
    }
    found
}

#[test]
fn the_identity_matcher_needs_a_whole_word() {
    let id = "ABCD12345";
    let name = "Some Title: The Sequel";
    for hit in [
        "boots ABCD12345 to the checkpoint",
        "manifest `ABCD12345.toml`",
        "(ABCD12345)",
    ] {
        assert!(names_identity(hit, id), "{hit:?} names the title");
    }
    assert!(names_identity(
        "the row for Some Title: The Sequel converges",
        name
    ));
    for miss in [
        "XABCD12345 is a different serial",
        "ABCD123456 has one more digit",
        "a mechanism paragraph naming no title",
    ] {
        assert!(
            !names_identity(miss, id),
            "{miss:?} does not name the title"
        );
    }
    assert!(!names_identity(
        "some title: the sequel in lower case is prose",
        name
    ));
    assert!(!names_identity("Some Title: The Sequels", name));
}

#[test]
fn a_firmware_exec_manifest_contributes_its_display_name_only() {
    let game = "[title]\ncontent_id = \"ABCD12345\"\nshort_name = \"x\"\ndisplay_name = \"Some Title\"\ndistribution = \"psn-hdd\"\n[checkpoint]\nkind = \"process-exit\"\n";
    assert_eq!(manifest_identities(game), ["ABCD12345", "Some Title"]);
    let firmware = "[title]\ncontent_id = \"SYS\"\ndisplay_name = \"System Software (sys)\"\ndistribution = \"firmware-exec\"\n[source]\nkind = \"firmware-exec\"\n";
    assert_eq!(manifest_identities(firmware), ["System Software (sys)"]);
}

#[test]
fn the_issue_reference_matcher_separates_tracker_numbers_from_the_look_alikes() {
    for hit in [
        "see #123 for the history",
        "(#45)",
        "filed as #4567.",
        "tracker #12:",
    ] {
        assert!(has_issue_reference(hit), "{hit:?} is a tracker reference");
    }
    for miss in [
        "DMA #1: 128 bytes",
        "#[derive(Debug)]",
        "renders as mod#0xHHHHHHHH",
        "#0x1234 is hex",
        "item #2 of the list",
        "the #12ab token",
        "no reference here",
    ] {
        assert!(
            !has_issue_reference(miss),
            "{miss:?} is not a tracker reference"
        );
    }
}

#[test]
fn the_tree_matcher_needs_a_path_boundary() {
    assert_eq!(
        cites_untracked_tree("see docs/dev/notes.md"),
        Some("docs/dev/")
    );
    assert_eq!(
        cites_untracked_tree("edit `.claude/settings.json`"),
        Some(".claude/")
    );
    assert_eq!(
        cites_untracked_tree("run scripts/cite.py"),
        Some("scripts/")
    );
    assert_eq!(
        cites_untracked_tree("per CLAUDE.local.md"),
        Some("CLAUDE.local.md")
    );
    assert_eq!(cites_untracked_tree("under mydocs/dev/x"), None);
    assert_eq!(cites_untracked_tree("the build-scripts/ directory"), None);
    assert_eq!(cites_untracked_tree("docs/architecture/boot.md"), None);
}

#[test]
fn every_untracked_tree_is_still_ignored() {
    let gitignore = read(&workspace_root().join(".gitignore"));
    let rules: Vec<&str> = gitignore.lines().map(str::trim).collect();
    for tree in UNTRACKED_TREES {
        let bare = tree.trim_end_matches('/');
        assert!(
            rules
                .iter()
                .any(|r| r.trim_start_matches('/').trim_end_matches('/') == bare),
            "{tree} is policed as untracked but .gitignore no longer lists it"
        );
    }
}

#[test]
fn mechanism_documents_name_no_title() {
    let root = workspace_root();
    let identities = title_identities(&root);
    assert!(
        identities.len() >= MIN_IDENTITIES,
        "gate went vacuous: only {} title identit(ies) read from title_manifests/",
        identities.len()
    );
    let docs = mechanism_docs(&root);
    assert!(
        docs.len() >= MIN_MECHANISM_DOCS,
        "gate went vacuous: only {} document(s) under docs/architecture and docs/concepts",
        docs.len()
    );
    let mut violations = Vec::new();
    for doc in &docs {
        for (n, line) in read(doc).lines().enumerate() {
            for identity in &identities {
                if names_identity(line, identity) {
                    let shown = doc.strip_prefix(&root).unwrap_or(doc);
                    violations.push(format!(
                        "  {}:{}  names {identity}\n",
                        shown.display(),
                        n + 1
                    ));
                    break;
                }
            }
        }
    }
    violations.sort();
    assert!(
        violations.is_empty(),
        "architecture and concepts documents describe mechanism; per-title state lives in \
         docs/titles.md:\n{}",
        violations.concat()
    );
}

#[test]
fn shipped_comments_carry_no_issue_reference() {
    let root = workspace_root();
    let files = shipped_rust(&root);
    let mut comment_lines = 0usize;
    let mut violations = Vec::new();
    for file in &files {
        if defines_a_rule(file) {
            continue;
        }
        for (n, line) in read(file).lines().enumerate() {
            let Some(body) = comment_body(line) else {
                continue;
            };
            comment_lines += 1;
            if has_issue_reference(body) {
                let shown = file.strip_prefix(&root).unwrap_or(file);
                violations.push(format!("  {}:{}\n", shown.display(), n + 1));
            }
        }
    }
    assert!(
        comment_lines >= MIN_COMMENT_LINES,
        "gate went vacuous: only {comment_lines} comment line(s) scanned across {} files",
        files.len()
    );
    violations.sort();
    assert!(
        violations.is_empty(),
        "{} comment(s) carry an issue reference:\n{}",
        violations.len(),
        violations.concat()
    );
}

#[test]
fn no_tracked_text_cites_a_path_git_does_not_track() {
    let root = workspace_root();
    let files = tracked_text_files(&root);
    assert!(
        files.len() >= MIN_TEXT_FILES,
        "gate went vacuous: only {} text file(s) walked",
        files.len()
    );
    let mut violations = Vec::new();
    for file in &files {
        let Ok(text) = fs::read_to_string(file) else {
            continue;
        };
        for (n, line) in text.lines().enumerate() {
            if let Some(tree) = cites_untracked_tree(line) {
                let shown = file.strip_prefix(&root).unwrap_or(file);
                violations.push(format!("  {}:{}  cites {tree}\n", shown.display(), n + 1));
            }
        }
    }
    violations.sort();
    assert!(
        violations.is_empty(),
        "{} line(s) point a reader at a path git does not track. State the fact inline and \
         delete the pointer:\n{}",
        violations.len(),
        violations.concat()
    );
}
