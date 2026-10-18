//! Convention guard: data and generated output carry a review decision.
//!
//! `.gitattributes` marks data `linguist-generated`, so GitHub collapses
//! it in a diff and a review lands on the code beside it. A file the
//! repository writes by hand carries `-linguist-generated` (unset), so
//! it stays expanded. This guard reads the attribute through
//! `git check-attr` and fails when:
//!
//! - a console capture under `tests/micro/*/ps3/`, an RPCS3 baseline
//!   under `tests/scenario_observations/*/`, or a `.tsv` / `.csv` table
//!   carries neither the attribute nor a named exemption;
//! - a table in `docs/lv2/` disagrees with the owner class the archive's
//!   README gives it: a curated or attributed table carries the
//!   attribute, or an extracted or generated one carries no decision;
//! - a tracked text file over [`LARGE_FILE_LINES`] lines carries no
//!   decision either way.
//!
//! The attribute is git's to resolve, so the guard runs git. Without git
//! on the path it fails, naming the command.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// A tracked text file longer than this needs a decision in
/// `.gitattributes`.
const LARGE_FILE_LINES: usize = 2000;

/// Bytes read to tell a binary file from a text one.
const BINARY_PROBE_BYTES: usize = 8192;

/// The attribute the guard reads.
const ATTRIBUTE: &str = "linguist-generated";

/// The archive README whose file table classes every `docs/lv2/` file.
const LV2_README: &str = "docs/lv2/README.md";

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("testkit manifest dir is two levels under the workspace root")
        .to_path_buf()
}

/// Run git in the workspace root, feeding `stdin`, and return stdout.
fn git(root: &Path, args: &[&str], stdin: &[u8]) -> Vec<u8> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|e| panic!("cannot run `git {}`: {e}", args.join(" ")));
    // git answers as it reads, so a writer that sends every byte before
    // reading fills git's output pipe and both sides wait. The input goes
    // in from its own thread while this one reads.
    let mut pipe = child.stdin.take().expect("piped stdin");
    let input = stdin.to_vec();
    let writer = std::thread::spawn(move || pipe.write_all(&input));
    let out = child
        .wait_with_output()
        .unwrap_or_else(|e| panic!("`git {}`: {e}", args.join(" ")));
    writer
        .join()
        .expect("stdin writer thread")
        .unwrap_or_else(|e| panic!("`git {}` stdin: {e}", args.join(" ")));
    assert!(
        out.status.success(),
        "`git {}` failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&out.stderr)
    );
    out.stdout
}

/// NUL-separated fields, empty trailing field dropped.
fn nul_fields(bytes: &[u8]) -> Vec<String> {
    bytes
        .split(|&b| b == 0)
        .filter(|f| !f.is_empty())
        .map(|f| String::from_utf8_lossy(f).into_owned())
        .collect()
}

/// `git check-attr -z` output, `<path> NUL <attribute> NUL <value> NUL`
/// per path, as path -> value.
fn parse_check_attr(bytes: &[u8]) -> BTreeMap<String, String> {
    let fields = nul_fields(bytes);
    assert_eq!(
        fields.len() % 3,
        0,
        "check-attr output is path/attr/value triples"
    );
    fields
        .chunks_exact(3)
        .map(|t| (t[0].clone(), t[2].clone()))
        .collect()
}

/// Every tracked path with its resolved attribute value: `true`,
/// `false`, `unset` or `unspecified`.
fn tracked_attributes(root: &Path) -> BTreeMap<String, String> {
    let listing = git(root, &["ls-files", "-z"], b"");
    let attrs = git(root, &["check-attr", "-z", "--stdin", ATTRIBUTE], &listing);
    parse_check_attr(&attrs)
}

/// Whether `path` is a recorded run: a file in a console capture
/// directory, `tests/micro/<probe>/ps3/...`, or an RPCS3 baseline,
/// `tests/scenario_observations/<scenario>/<file>`.
fn is_capture(path: &str) -> bool {
    let parts: Vec<&str> = path.split('/').collect();
    matches!(
        parts.as_slice(),
        ["tests", "micro", _, "ps3", _, ..] | ["tests", "scenario_observations", _, _]
    )
}

fn is_table(path: &str) -> bool {
    path.ends_with(".tsv") || path.ends_with(".csv")
}

/// The file rows of the archive README's table, `| \`<file>\` | <class> |`,
/// as repository path -> owner class.
fn lv2_owner_classes(readme: &str) -> BTreeMap<String, String> {
    readme
        .lines()
        .filter_map(|line| {
            let mut cells = line.split('|').map(str::trim).skip(1);
            let file = cells.next()?.strip_prefix('`')?.strip_suffix('`')?;
            let class = cells.next()?;
            matches!(class, "extracted" | "generated" | "curated" | "attributed")
                .then(|| (format!("docs/lv2/{file}"), class.to_string()))
        })
        .collect()
}

/// The line count of a tracked text file, or `None` for a binary one.
fn text_lines(path: &Path) -> Option<usize> {
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let probe = &bytes[..bytes.len().min(BINARY_PROBE_BYTES)];
    (!probe.contains(&0)).then(|| bytes.iter().filter(|&&b| b == b'\n').count())
}

#[test]
fn the_check_attr_reader_pairs_each_path_with_its_value() {
    let out = b"a.tsv\0linguist-generated\0true\0b.rs\0linguist-generated\0unspecified\0";
    let parsed = parse_check_attr(out);
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed["a.tsv"], "true");
    assert_eq!(parsed["b.rs"], "unspecified");
}

#[test]
fn the_capture_matcher_takes_the_recorded_runs_and_nothing_beside_them() {
    assert!(is_capture(
        "tests/micro/spu_events/ps3/cech20-cex-493/observation.json"
    ));
    assert!(!is_capture("tests/micro/spu_events/ppu/main.c"));
    assert!(!is_capture("tests/micro/ps3_gaps.tsv"));
    assert!(!is_capture("tests/micro/spu_events/ps3"));
    assert!(is_capture(
        "tests/scenario_observations/spu_events/rpcs3_llvm.json"
    ));
    assert!(!is_capture("tests/scenario_observations/README.md"));
}

#[test]
fn the_readme_reader_takes_the_file_rows_and_their_classes() {
    let readme = "\
| File | Owner class | Regenerate | Gate |
| --- | --- | --- | --- |
| `tables/behavior.tsv` | curated | written by hand | `x` |
| `census/fw-4.93.tsv` | extracted | `cmd` | `y` |
| Class | Meaning |
| generated | Rendered from code. |
";
    let classes = lv2_owner_classes(readme);
    assert_eq!(classes.len(), 2);
    assert_eq!(classes["docs/lv2/tables/behavior.tsv"], "curated");
    assert_eq!(classes["docs/lv2/census/fw-4.93.tsv"], "extracted");
}

#[test]
fn every_capture_and_table_is_collapsed_or_exempted_by_name() {
    let attrs = tracked_attributes(&workspace_root());
    let data: Vec<(&String, &String)> = attrs
        .iter()
        .filter(|(path, _)| is_capture(path) || is_table(path))
        .collect();
    assert!(
        data.len() > 500,
        "only {} captures and tables found: the matcher or the listing broke",
        data.len()
    );
    let undecided: Vec<String> = data
        .iter()
        .filter(|(_, value)| !matches!(value.as_str(), "true" | "unset"))
        .map(|(path, value)| format!("{path}: {value}"))
        .collect();
    assert!(
        undecided.is_empty(),
        "data neither collapsed (`linguist-generated=true`) nor exempted by name \
         (`-linguist-generated`) in .gitattributes:\n{}",
        undecided.join("\n")
    );
}

#[test]
fn every_lv2_archive_file_follows_its_owner_class() {
    let root = workspace_root();
    let readme =
        fs::read_to_string(root.join(LV2_README)).unwrap_or_else(|e| panic!("{LV2_README}: {e}"));
    let classes = lv2_owner_classes(&readme);
    assert!(
        classes.len() > 100,
        "only {} archive rows read from {LV2_README}: the table reader broke",
        classes.len()
    );
    let attrs = tracked_attributes(&root);
    let wrong: Vec<String> = classes
        .iter()
        .filter_map(|(path, class)| {
            let value = attrs.get(path).map_or("untracked", String::as_str);
            let fits = match class.as_str() {
                // A table written by hand stays expanded.
                "curated" | "attributed" => matches!(value, "unset" | "false"),
                // Output a command writes carries an explicit decision.
                _ => matches!(value, "true" | "false"),
            };
            (!fits).then(|| format!("{path} ({class}): {value}"))
        })
        .collect();
    assert!(
        wrong.is_empty(),
        "archive files whose .gitattributes decision disagrees with their owner \
         class in {LV2_README}:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn every_large_text_file_carries_a_decision() {
    let root = workspace_root();
    let attrs = tracked_attributes(&root);
    let mut large = 0;
    let mut undecided = Vec::new();
    for (path, value) in &attrs {
        let Some(lines) = text_lines(&root.join(path)) else {
            continue;
        };
        if lines <= LARGE_FILE_LINES {
            continue;
        }
        large += 1;
        if value == "unspecified" {
            undecided.push(format!("{path}: {lines} lines"));
        }
    }
    assert!(
        large > 0,
        "no tracked file exceeds {LARGE_FILE_LINES} lines: the scan broke"
    );
    assert!(
        undecided.is_empty(),
        "text files over {LARGE_FILE_LINES} lines with no `linguist-generated` decision \
         in .gitattributes (collapse data, mark hand-written files \
         `-linguist-generated`):\n{}",
        undecided.join("\n")
    );
}
