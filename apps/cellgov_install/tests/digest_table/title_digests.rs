//! The title-digest manifest's well-formedness, over the committed rows
//! and over each refused shape.

#[path = "../common/title_digests.rs"]
mod manifest;
pub use manifest::*;

/// The message [`assert_well_formed`] refused `rows` with.
///
/// # Panics
///
/// If the rows were accepted, or the refusal carried a non-string
/// payload.
fn refusal_message(rows: &str) -> String {
    let titles = parse(rows, "t");
    let payload = std::panic::catch_unwind(|| assert_well_formed(&titles))
        .err()
        .unwrap_or_else(|| panic!("assert_well_formed accepted {rows:?}"));
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| panic!("the refusal of {rows:?} carried a non-string payload"))
}

const A64: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const B64: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

#[test]
fn the_committed_manifest_parses_and_every_row_is_well_formed() {
    let titles = load();
    assert_well_formed(&titles);
    assert!(
        titles.iter().any(TitleDigest::is_rpcs3_reference),
        "{} pins no npdrm row, so nothing in it is held against RPCS3",
        manifest_path().display()
    );
}

/// One `app` row with `unmasked_sha256 = A64`, plus `extra` lines.
fn app_row(extra: &str) -> String {
    format!(
        "[[title]]\ncontent_id = \"X\"\ndisplay = \"X\"\nkey = \"app\"\n\
         unmasked_sha256 = \"{A64}\"\n{extra}"
    )
}

#[test]
fn a_repeated_content_id_is_named_rather_than_compared_twice() {
    let rows = format!(
        "{}[[title]]\ncontent_id = \"X\"\ndisplay = \"X2\"\nkey = \"app\"\n\
         unmasked_sha256 = \"{B64}\"\n",
        app_row("")
    );
    let msg = refusal_message(&rows);
    assert!(msg.contains("more than once"), "got {msg:?}");
}

#[test]
fn an_unknown_key_is_named_rather_than_passed_over() {
    let rows = format!(
        "[[title]]\ncontent_id = \"X\"\ndisplay = \"X\"\nkey = \"disc\"\n\
         unmasked_sha256 = \"{A64}\"\n"
    );
    let msg = refusal_message(&rows);
    assert!(msg.contains("unknown key"), "got {msg:?}");
}

#[test]
fn an_npdrm_row_missing_its_masked_hash_is_rejected() {
    let rows = format!(
        "[[title]]\ncontent_id = \"X\"\ndisplay = \"X\"\nkey = \"npdrm\"\n\
         rap_filename = \"x.rap\"\nunmasked_sha256 = \"{A64}\"\n"
    );
    let msg = refusal_message(&rows);
    assert!(msg.contains("requires masked_sha256"), "got {msg:?}");
}

#[test]
fn an_npdrm_row_missing_its_rap_is_rejected() {
    let rows = format!(
        "[[title]]\ncontent_id = \"X\"\ndisplay = \"X\"\nkey = \"npdrm\"\n\
         unmasked_sha256 = \"{A64}\"\nmasked_sha256 = \"{B64}\"\n"
    );
    let msg = refusal_message(&rows);
    assert!(msg.contains("requires rap_filename"), "got {msg:?}");
}

/// An npdrm row whose two hashes coincide makes the masked fallback a
/// second copy of the unmasked check.
#[test]
fn an_npdrm_row_whose_masked_hash_repeats_the_unmasked_one_is_rejected() {
    let rows = format!(
        "[[title]]\ncontent_id = \"X\"\ndisplay = \"X\"\nkey = \"npdrm\"\n\
         rap_filename = \"x.rap\"\nunmasked_sha256 = \"{A64}\"\n\
         masked_sha256 = \"{A64}\"\n"
    );
    let msg = refusal_message(&rows);
    assert!(msg.contains("equals unmasked_sha256"), "got {msg:?}");
}

#[test]
fn an_app_row_carrying_a_column_its_key_never_reads_is_rejected() {
    for extra in [
        format!("masked_sha256 = \"{B64}\"\n"),
        "rap_filename = \"x.rap\"\n".to_owned(),
    ] {
        let msg = refusal_message(&app_row(&extra));
        assert!(msg.contains("never read"), "extra {extra:?}: got {msg:?}");
    }
}

#[test]
fn a_hash_that_is_not_64_hex_characters_is_rejected() {
    let all_z = A64.replace('a', "z");
    for bad in ["deadbeef", &A64[..63], &format!("{A64}a"), &all_z] {
        let rows = format!(
            "[[title]]\ncontent_id = \"X\"\ndisplay = \"X\"\nkey = \"app\"\n\
             unmasked_sha256 = \"{bad}\"\n"
        );
        let msg = refusal_message(&rows);
        assert!(
            msg.contains("hex must be 64 chars") || msg.contains("invalid hex byte"),
            "hash {bad:?}: got {msg:?}"
        );
    }
}

#[test]
fn a_manifest_with_no_rows_is_rejected() {
    let msg = refusal_message("title = []\n");
    assert!(msg.contains("at least one"), "got {msg:?}");
}
