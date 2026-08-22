//! Committed RPCS3 reference digests, read by the parity tests.
//!
//! What RPCS3 produces for a given input is captured once into
//! `tests/fixtures/rpcs3_digests/digests.txt` and committed, so a
//! parity test needs no RPCS3 install tree at run time.

// Each integration test binary compiles this module separately and
// uses a different subset of it.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Workspace root, resolved from this crate's manifest dir.
pub fn workspace_root() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p
}

/// Parse the committed digest table, keyed as in the file.
///
/// # Panics
///
/// If the table is missing or a data line is malformed. It is a
/// committed fixture: absence means a broken checkout, not an
/// optional corpus, so it fails rather than skips.
pub fn table() -> BTreeMap<String, String> {
    let path = workspace_root().join("tests/fixtures/rpcs3_digests/digests.txt");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "committed reference digests unreadable at {}: {e}",
            path.display()
        )
    });
    parse_table(&text, &path.display().to_string())
}

/// Parse the digest table's `text`; `origin` names it in diagnostics.
///
/// # Panics
///
/// If a data line is malformed, its digest column is not 64 hex
/// characters, or a key repeats. Re-blessing pastes fresh rows into
/// the file, so a duplicate is an operator slip that must be named
/// rather than resolved by insertion order.
fn parse_table(text: &str, origin: &str) -> BTreeMap<String, String> {
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for (n, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut cols = line.split_whitespace();
        let (Some(sha), Some(key)) = (cols.next(), cols.next()) else {
            panic!("{origin}:{}: malformed digest line {line:?}", n + 1);
        };
        assert!(
            sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit()),
            "{origin}:{}: digest column {sha:?} is not 64 hex characters; \
             columns are <sha256>  <key>  <bytes>",
            n + 1
        );
        // The comparison side formats lowercase hex; fold case here so
        // an uppercase row is a match rather than a phantom divergence.
        let sha = sha.to_ascii_lowercase();
        if let Some(prev) = out.insert(key.to_string(), sha.clone()) {
            panic!(
                "{origin}:{}: key {key:?} appears more than once \
                 (first {prev}, then {sha}); delete the stale row rather \
                 than leaving insertion order to pick",
                n + 1
            );
        }
    }
    assert!(!out.is_empty(), "{origin} lists no digests");
    out
}

/// Lowercase hex SHA-256 of `path`'s bytes.
pub fn sha256_file(path: &Path) -> String {
    use sha2::{Digest, Sha256};
    let bytes =
        std::fs::read(path).unwrap_or_else(|e| panic!("read {} failed: {e}", path.display()));
    let mut h = Sha256::new();
    h.update(&bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn sha256_bytes(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(bytes);
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

const A64: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B64: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// The message [`parse_table`] refused `text` with.
///
/// # Panics
///
/// If `text` parsed, or the refusal carried a non-string payload.
/// Asserting on the message keeps a test from passing on an unrelated
/// panic -- an index slip refuses the input too, but for the wrong
/// reason.
fn refusal_message(text: &str) -> String {
    let payload = std::panic::catch_unwind(|| parse_table(text, "t"))
        .err()
        .unwrap_or_else(|| panic!("parse_table accepted {text:?}"));
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| panic!("refusal of {text:?} carried a non-string payload"))
}

#[test]
fn the_committed_table_parses_and_lists_every_key_the_parity_tests_read() {
    let t = table();
    for stem in [
        "libaudio",
        "libfs",
        "libgcm_sys",
        "libio",
        "liblv2",
        "libnet",
        "libnetctl",
        "libspurs_jq",
        "libsync2",
        "libsysmodule",
        "libsysutil",
        "libsysutil_np",
    ] {
        assert!(
            t.contains_key(&format!("decrypted_masked/{stem}")),
            "digests.txt is missing decrypted_masked/{stem}"
        );
    }
    for id in ["NPUA80001", "NPUA80068", "BCES00664"] {
        assert!(
            t.contains_key(&format!("eboot/{id}")),
            "digests.txt is missing eboot/{id}"
        );
    }
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
    assert_eq!(parse_table(&text, "t")["eboot/X"], A64);
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
    assert_eq!(t["eboot/X"], A64);
    assert_eq!(t["eboot/Y"], B64);
}

#[test]
fn a_line_carrying_only_a_digest_is_rejected_rather_than_silently_dropped() {
    let msg = refusal_message(&format!("{A64}\n"));
    assert!(msg.contains("malformed digest line"), "got {msg:?}");
}

#[test]
fn a_table_with_no_data_rows_is_rejected() {
    let msg = refusal_message("# only a comment\n");
    assert!(msg.contains("lists no digests"), "got {msg:?}");
}
