//! The data-free halves of the data-feature suites: the NPDRM witness
//! manifest's parser and the dump directory's package pick, each
//! driven with built inputs.
//!
//! The suites that read the external data stay behind their features;
//! these checks run on a fresh clone.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use std::path::Path;

#[path = "common/dump_pick.rs"]
mod dump_pick;
#[path = "common/npdrm_manifest.rs"]
mod npdrm_manifest;
#[path = "common/scratch.rs"]
mod scratch;

use dump_pick::only_with_ext;
use npdrm_manifest::parse;

/// The message `f` panicked with.
fn panic_message(f: impl FnOnce() + std::panic::UnwindSafe, what: &str) -> String {
    let payload = std::panic::catch_unwind(f)
        .err()
        .unwrap_or_else(|| panic!("{what} was accepted"));
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| panic!("the refusal of {what} carried a non-string payload"))
}

/// The message [`parse`] refused `text` with.
fn refusal_message(text: &str) -> String {
    panic_message(
        || {
            parse(text, "t");
        },
        &format!("{text:?}"),
    )
}

#[test]
fn a_manifest_with_no_vectors_is_rejected() {
    let msg = refusal_message("vector = []\n");
    assert!(msg.contains("pins no [[vector]]"), "got {msg:?}");
}

/// One well-formed row pinning `rap`, plus `extra` lines.
fn row(rap: &str, extra: &str) -> String {
    format!(
        "[[vector]]\nrap = \"{rap}\"\nklicensee = \"{}\"\n{extra}",
        "a".repeat(32)
    )
}

#[test]
fn an_unknown_field_is_named_rather_than_passed_over() {
    let msg = refusal_message(&row("x.rap", "ebot = \"e.bin\"\n"));
    assert!(msg.contains("unknown field"), "row level: got {msg:?}");
    let msg = refusal_message(&format!("vectors = []\n{}", row("x.rap", "")));
    assert!(msg.contains("unknown field"), "top level: got {msg:?}");
}

#[test]
fn a_repeated_rap_is_named_rather_than_derived_twice() {
    let text = format!("{}{}", row("x.rap", ""), row("x.rap", ""));
    let msg = refusal_message(&text);
    assert!(msg.contains("more than once"), "got {msg:?}");
}

#[test]
fn a_path_that_is_not_relative_under_the_vfs_is_rejected() {
    for bad in ["../x.rap", "/x.rap", "a/../../x.rap"] {
        let msg = refusal_message(&row(bad, ""));
        assert!(
            msg.contains("leaves the workspace"),
            "rap {bad:?}: got {msg:?}"
        );
        let msg = refusal_message(&row("x.rap", &format!("eboot = \"{bad}\"\n")));
        assert!(
            msg.contains("leaves the workspace"),
            "eboot {bad:?}: got {msg:?}"
        );
    }
    let msg = refusal_message(&row("", ""));
    assert!(msg.contains("is empty"), "got {msg:?}");
    let msg = refusal_message(&row("x.rap", "eboot = \"\"\n"));
    assert!(msg.contains("is empty"), "got {msg:?}");
}

#[test]
fn a_klicensee_that_is_not_32_hex_characters_is_rejected() {
    for bad in ["deadbeef", &"a".repeat(31), &"z".repeat(32)] {
        let text = format!("[[vector]]\nrap = \"x.rap\"\nklicensee = \"{bad}\"\n");
        let msg = refusal_message(&text);
        assert!(
            msg.contains("hex must be 32 chars") || msg.contains("invalid hex byte"),
            "klicensee {bad:?}: got {msg:?}"
        );
    }
}

#[test]
fn a_well_formed_manifest_parses_every_row() {
    let text = format!(
        "{}{}",
        row("a.rap", "eboot = \"e.bin\"\n"),
        row("b.rap", "")
    );
    let rows = parse(&text, "t");
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].eboot.as_deref(), Some("e.bin"));
    assert_eq!(rows[1].eboot, None);
}

#[test]
fn only_with_ext_returns_the_single_match_and_names_every_other_outcome() {
    let scratch = scratch::scratch_labeled("only_with_ext");
    let dir = scratch.join("dump");
    std::fs::create_dir_all(&dir).unwrap();

    // Both payload shapes are read: a `&str` refusal reaching the
    // `None` arm would be reported as acceptance.
    let refusal = |dir: &Path| {
        panic_message(
            || {
                only_with_ext(dir, "pkg");
            },
            &dir.display().to_string(),
        )
    };

    let msg = refusal(&dir);
    assert!(msg.contains("no .pkg in"), "got {msg:?}");

    std::fs::write(dir.join("flow.PKG"), b"x").unwrap();
    std::fs::write(dir.join("flow.rap"), b"x").unwrap();
    assert_eq!(only_with_ext(&dir, "pkg"), dir.join("flow.PKG"));

    std::fs::write(dir.join("other.pkg"), b"x").unwrap();
    let msg = refusal(&dir);
    assert!(msg.contains("holds 2 .pkg files"), "got {msg:?}");
    assert!(
        msg.contains("flow.PKG") && msg.contains("other.pkg"),
        "got {msg:?}"
    );

    let msg = refusal(&scratch.join("absent"));
    assert!(msg.contains("read dir"), "got {msg:?}");
}
