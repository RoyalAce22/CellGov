//! `table.rs` drift gate: the committed file is the rendering of `table.tsv`.
//!
//! Regenerate with:
//!
//! ```text
//! cargo test -p cellgov_ps3_abi --lib -- --ignored regenerate_nid_table
//! ```

use std::path::PathBuf;

const TSV: &str = include_str!("../table.tsv");
const COMMITTED: &str = include_str!("../table.rs");

const HEADER: &str = "nid\tmodule\tname";

const REGENERATE: &str =
    "regenerate with: cargo test -p cellgov_ps3_abi --lib -- --ignored regenerate_nid_table";

fn table_rs_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src/nid/table.rs")
}

/// The `(nid, module, name)` rows of a data file.
///
/// # Panics
///
/// Panics on a row that:
/// - the rendered string literal could not carry, or
/// - `lookup` could not binary-search.
fn parse(tsv: &str) -> Vec<(u32, &str, &str)> {
    let mut lines = tsv.lines();
    assert_eq!(
        lines.next(),
        Some(HEADER),
        "table.tsv starts with the column header"
    );
    let mut rows: Vec<(u32, &str, &str)> = Vec::new();
    for (i, line) in lines.enumerate() {
        let lineno = i + 2;
        let fields: Vec<&str> = line.split('\t').collect();
        let [nid, module, name] = fields[..] else {
            panic!("table.tsv line {lineno}: expected 3 tab-separated fields, got {line:?}");
        };
        assert!(
            nid.len() == 8
                && nid
                    .bytes()
                    .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()),
            "table.tsv line {lineno}: nid {nid:?} is not 8 lower-case hex digits"
        );
        let nid =
            u32::from_str_radix(nid, 16).unwrap_or_else(|e| panic!("table.tsv line {lineno}: {e}"));
        for field in [module, name] {
            assert!(
                !field.contains(['"', '\\']),
                "table.tsv line {lineno}: {field:?} holds a quote or backslash"
            );
            // `lines()` strips a line-final CR. A CR inside a field reaches
            // the literal, and rustc rejects a bare CR there.
            assert!(
                !field.contains('\r'),
                "table.tsv line {lineno}: {field:?} holds a bare carriage return"
            );
        }
        assert!(!name.is_empty(), "table.tsv line {lineno}: empty name");
        if let Some(&(prev, _, prev_name)) = rows.last() {
            assert!(
                prev < nid,
                "table.tsv line {lineno}: 0x{nid:08x} {name:?} does not follow 0x{prev:08x} \
                 {prev_name:?}; rows are strictly ascending by nid"
            );
        }
        rows.push((nid, module, name));
    }
    rows
}

fn render(rows: &[(u32, &str, &str)]) -> String {
    let mut out = String::from(
        "//! `NID_TABLE`, rendered from `table.tsv` beside this file.\n\
         //!\n\
         //! Generated: edit the data file, then regenerate with\n\
         //! `cargo test -p cellgov_ps3_abi --lib -- --ignored regenerate_nid_table`.\n\
         //! `nid::table_gen_tests::committed_table_matches_tsv` fails on drift.\n\
         \n\
         /// `(NID, module, function)` sorted by NID for binary search.\n\
         #[rustfmt::skip]\n\
         pub(super) static NID_TABLE: &[(u32, &str, &str)] = &[\n",
    );
    for (nid, module, name) in rows {
        out.push_str(&format!("    (0x{nid:08x}, \"{module}\", \"{name}\"),\n"));
    }
    out.push_str("];\n");
    out
}

/// The first line where two texts differ, as `(line number, left, right)`.
///
/// `None` means the two are byte-identical. When every line agrees but
/// the bytes do not, the two differ only in final newlines, and the
/// result names the line after the last.
fn first_difference<'a>(a: &'a str, b: &'a str) -> Option<(usize, &'a str, &'a str)> {
    let mut la = a.lines();
    let mut lb = b.lines();
    let mut n = 0;
    loop {
        n += 1;
        match (la.next(), lb.next()) {
            (None, None) => break,
            (l, r) if l == r => {}
            (l, r) => {
                return Some((
                    n,
                    l.unwrap_or("<end of file>"),
                    r.unwrap_or("<end of file>"),
                ))
            }
        }
    }
    if a == b {
        return None;
    }
    Some((n, final_newlines(a), final_newlines(b)))
}

fn final_newlines(s: &str) -> &'static str {
    match s.len() - s.trim_end_matches('\n').len() {
        0 => "<no final newline>",
        1 => "<one final newline>",
        _ => "<more than one final newline>",
    }
}

#[test]
fn a_two_row_file_renders_one_tuple_per_row() {
    let tsv = "nid\tmodule\tname\n000e53cc\tsceNp\tsceNpManagerSubSignout\n003395d9\t\t_Feraise\n";
    let rendered = render(&parse(tsv));
    assert!(rendered.starts_with("//! `NID_TABLE`"));
    assert!(rendered.ends_with(
        "= &[\n    (0x000e53cc, \"sceNp\", \"sceNpManagerSubSignout\"),\n    (0x003395d9, \"\", \"_Feraise\"),\n];\n"
    ));
}

#[test]
#[should_panic(expected = "strictly ascending")]
fn an_out_of_order_row_is_refused() {
    parse("nid\tmodule\tname\n003395d9\t\t_Feraise\n000e53cc\tsceNp\tsceNpManagerSubSignout\n");
}

#[test]
#[should_panic(expected = "holds a quote or backslash")]
fn a_name_the_literal_could_not_carry_is_refused() {
    parse("nid\tmodule\tname\n000e53cc\tsceNp\ta\"b\n");
}

#[test]
#[should_panic(expected = "holds a bare carriage return")]
fn a_carriage_return_inside_a_field_is_refused() {
    parse("nid\tmodule\tname\n000e53cc\tsceNp\ta\rb\n");
}

#[test]
fn a_line_final_carriage_return_is_not_part_of_the_name() {
    let rows = parse("nid\tmodule\tname\r\n000e53cc\tsceNp\tsceNpManagerSubSignout\r\n");
    assert_eq!(rows, vec![(0x000e_53cc, "sceNp", "sceNpManagerSubSignout")]);
}

#[test]
#[should_panic(expected = "strictly ascending")]
fn a_duplicate_key_is_refused() {
    parse("nid\tmodule\tname\n000e53cc\tsceNp\ta\n000e53cc\tsceNp\tb\n");
}

#[test]
#[should_panic(expected = "is not 8 lower-case hex digits")]
fn an_upper_case_key_is_refused() {
    parse("nid\tmodule\tname\n000E53CC\tsceNp\ta\n");
}

#[test]
#[should_panic(expected = "is not 8 lower-case hex digits")]
fn a_nine_digit_key_is_refused() {
    parse("nid\tmodule\tname\n0000e53cc\tsceNp\ta\n");
}

#[test]
#[should_panic(expected = "empty name")]
fn an_empty_name_is_refused() {
    parse("nid\tmodule\tname\n000e53cc\tsceNp\t\n");
}

#[test]
#[should_panic(expected = "expected 3 tab-separated fields")]
fn a_tab_inside_a_name_is_refused() {
    parse("nid\tmodule\tname\n000e53cc\tsceNp\ta\tb\n");
}

#[test]
#[should_panic(expected = "expected 3 tab-separated fields")]
fn a_trailing_blank_line_is_refused() {
    parse("nid\tmodule\tname\n000e53cc\tsceNp\ta\n\n");
}

#[test]
#[should_panic(expected = "starts with the column header")]
fn a_byte_order_mark_is_refused() {
    parse("\u{feff}nid\tmodule\tname\n000e53cc\tsceNp\ta\n");
}

#[test]
#[should_panic(expected = "starts with the column header")]
fn an_empty_file_is_refused() {
    parse("");
}

#[test]
fn a_difference_only_in_final_newlines_is_drift() {
    assert_eq!(first_difference("a\nb\n", "a\nb\n"), None);
    assert_eq!(
        first_difference("a\nb\n", "a\nb"),
        Some((3, "<one final newline>", "<no final newline>"))
    );
    assert_eq!(
        first_difference("a\nb\n", "a\nb\n\n"),
        Some((3, "<end of file>", ""))
    );
}

#[test]
fn committed_table_matches_tsv() {
    let rendered = render(&parse(TSV));
    let committed = COMMITTED.replace("\r\n", "\n");
    if let Some((line, left, right)) = first_difference(&committed, &rendered) {
        panic!(
            "src/nid/table.rs is stale at line {line}:\n  committed: {left}\n  rendered:  {right}\n{REGENERATE}"
        );
    }
}

#[test]
#[ignore = "writes src/nid/table.rs; run after editing table.tsv"]
fn regenerate_nid_table() {
    let path = table_rs_path();
    std::fs::write(&path, render(&parse(TSV)))
        .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
}
