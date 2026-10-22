//! The digest-table parser's own tests, and the check that the committed
//! table lists every key the parity suites read.

#[path = "../common/digests.rs"]
mod table;
pub use table::*;

const A64: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B64: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// The message `f` panicked with; `what` names the input in failures.
///
/// # Panics
///
/// If `f` returned, or the refusal carried a non-string payload.
fn refusal<T>(what: &str, f: impl FnOnce() -> T + std::panic::UnwindSafe) -> String {
    let payload = std::panic::catch_unwind(f)
        .err()
        .unwrap_or_else(|| panic!("{what:?} was accepted"));
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| panic!("refusal of {what:?} carried a non-string payload"))
}

/// The message [`parse_table`] refused `text` with.
fn refusal_message(text: &str) -> String {
    refusal(text, || parse_table(text, "t"))
}

/// Rows whose key no parity suite reads yet.
///
/// `eboot/NPUA80068` was captured by the re-bless recipe alongside the
/// two the suites do read, but no suite installs SSHD's PKG, so its
/// install path is not held against RPCS3.
const CAPTURED_BUT_UNREAD: &[&str] = &["eboot/NPUA80068"];

#[test]
fn the_committed_table_parses_and_lists_every_key_a_parity_suite_reads() {
    let t = table();
    let firmware = reference_firmware(&t);
    for stem in FIRMWARE_MODULES {
        for key in [
            format!("decrypted_masked/{stem}"),
            format!("sprx/{firmware}/{stem}"),
        ] {
            assert!(t.contains_key(&key), "digests.txt is missing {key}");
        }
    }
    // NPUA80001 is read by parity_pkg, BCES00664 by parity_disc.
    for id in ["NPUA80001", "BCES00664"] {
        assert!(
            t.contains_key(&format!("eboot/{id}")),
            "digests.txt is missing eboot/{id}"
        );
    }
    for key in CAPTURED_BUT_UNREAD {
        assert!(t.contains_key(*key), "digests.txt is missing {key}");
    }
    // The byte count is what the parity suites hold a produced length
    // against, so a row carrying zero would assert nothing.
    for (key, row) in &t {
        assert!(row.bytes > 0, "digests.txt row {key} claims 0 bytes");
    }
}

#[test]
fn no_committed_module_row_names_a_module_outside_the_list() {
    let t = table();
    let sprx_prefix = format!("sprx/{}/", reference_firmware(&t));
    let mut module_rows = 0usize;
    for key in t.keys() {
        let stem = key
            .strip_prefix("decrypted_masked/")
            .or_else(|| key.strip_prefix(sprx_prefix.as_str()));
        if let Some(stem) = stem {
            assert!(
                FIRMWARE_MODULES.contains(&stem),
                "digests.txt row {key} names a module outside FIRMWARE_MODULES"
            );
            module_rows += 1;
        }
    }
    assert!(module_rows > 0, "digests.txt carries no module row");
}

#[test]
fn a_repeated_key_is_named_rather_than_resolved_by_insertion_order() {
    let text = format!("{A64}  eboot/X  4\n{B64}  eboot/X  4\n");
    let msg = refusal_message(&text);
    assert!(msg.contains("appears more than once"), "got {msg:?}");
}

#[test]
fn an_uppercase_digest_row_folds_to_the_lowercase_the_comparison_produces() {
    let text = format!("{}  eboot/X  4\n", A64.to_ascii_uppercase());
    assert_eq!(parse_table(&text, "t")["eboot/X"].sha256, A64);
}

#[test]
fn a_digest_column_that_is_not_64_hex_characters_is_rejected() {
    // The last case is the one the length half of the check cannot
    // see: 64 characters, none of them hex.
    let non_hex_64 = "z".repeat(64);
    for bad in [
        "deadbeef",
        "zz",
        &A64[..63],
        &format!("{A64}a"),
        &non_hex_64,
    ] {
        let text = format!("{bad}  eboot/X  4\n");
        let msg = refusal_message(&text);
        assert!(
            msg.contains("is not 64 hex characters"),
            "digest column {bad:?}: got {msg:?}"
        );
    }
}

#[test]
fn crlf_rows_and_trailing_whitespace_parse_the_same_as_bare_newlines() {
    let text = format!("# c\r\n{A64}  eboot/X  4  \r\n\r\n{B64}\teboot/Y\t8\r\n");
    let t = parse_table(&text, "t");
    assert_eq!(t["eboot/X"].sha256, A64);
    assert_eq!(t["eboot/X"].bytes, 4);
    assert_eq!(t["eboot/Y"].sha256, B64);
    assert_eq!(t["eboot/Y"].bytes, 8);
}

#[test]
fn a_line_carrying_only_a_digest_is_rejected_rather_than_silently_dropped() {
    let msg = refusal_message(&format!("{A64}\n"));
    assert!(msg.contains("malformed digest line"), "got {msg:?}");
}

#[test]
fn a_row_missing_the_byte_count_is_rejected_rather_than_read_as_a_digest_only_row() {
    let msg = refusal_message(&format!("{A64}  eboot/X\n"));
    assert!(msg.contains("malformed digest line"), "got {msg:?}");
}

#[test]
fn a_byte_count_that_is_not_a_number_is_named() {
    for bad in ["4a", "-1", "0x10", "4.0"] {
        let msg = refusal_message(&format!("{A64}  eboot/X  {bad}\n"));
        assert!(
            msg.contains("is not a number"),
            "byte count {bad:?}: got {msg:?}"
        );
    }
}

#[test]
fn a_row_carrying_a_fourth_column_is_rejected() {
    let msg = refusal_message(&format!("{A64}  eboot/X  4  extra\n"));
    assert!(msg.contains("fourth column"), "got {msg:?}");
}

#[test]
fn a_table_with_no_data_rows_is_rejected() {
    let msg = refusal_message("# only a comment\n");
    assert!(msg.contains("lists no digests"), "got {msg:?}");
}

mod reference_firmware_rows {
    use super::{parse_table, reference_firmware, refusal, A64, B64};

    /// The message [`reference_firmware`] refused the table in `text` with.
    fn refusal_of(text: &str) -> String {
        let t = parse_table(text, "t");
        refusal(text, || reference_firmware(&t))
    }

    #[test]
    fn the_version_comes_from_the_sprx_rows_alone() {
        let text = format!(
            "{A64}  eboot/X  4\n{A64}  decrypted_masked/libfs  8\n\
             {B64}  sprx/4.93/libfs  4\n{B64}  sprx/4.93/libio  4\n"
        );
        assert_eq!(reference_firmware(&parse_table(&text, "t")), "4.93");
    }

    #[test]
    fn sprx_rows_naming_two_versions_are_refused() {
        let msg = refusal_of(&format!(
            "{A64}  sprx/4.92/libfs  4\n{B64}  sprx/4.93/libio  4\n"
        ));
        assert!(msg.contains("\"4.92\", \"4.93\""), "got {msg:?}");
    }

    #[test]
    fn a_table_with_no_sprx_row_records_no_firmware() {
        let msg = refusal_of(&format!("{A64}  decrypted_masked/libfs  8\n"));
        assert!(
            msg.contains("no sprx/<version>/<module> row"),
            "got {msg:?}"
        );
    }

    #[test]
    fn an_sprx_key_that_is_not_version_then_module_is_refused() {
        for key in ["sprx/libfs", "sprx//libfs", "sprx/4.93/", "sprx/4.93/a/b"] {
            let msg = refusal_of(&format!("{A64}  {key}  4\n"));
            assert!(
                msg.contains("does not name sprx/<version>/<module>"),
                "key {key:?}: got {msg:?}"
            );
        }
    }
}
