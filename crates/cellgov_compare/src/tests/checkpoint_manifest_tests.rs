use super::*;
use cellgov_testkit::scratch::scratch_labeled;

fn parse(text: &str) -> CheckpointManifest {
    CheckpointManifest::from_toml(text).expect("parses")
}

#[test]
fn hex_strings_parse_with_or_without_the_prefix() {
    let m = parse(
        r#"
        [[regions]]
        name = "code"
        addr = "0x10000"
        size = "0x800000"

        [[regions]]
        name = "r"
        addr = "1000"
        size = "10"
        "#,
    );
    assert_eq!(m.regions.len(), 2);
    assert_eq!(m.regions[0].addr, 0x10000);
    assert_eq!(m.regions[0].size, 0x80_0000);
    assert_eq!(
        m.regions[0].space, 0,
        "an absent space field means the boot space"
    );
    assert_eq!(m.regions[1].addr, 0x1000);
    assert_eq!(m.regions[1].size, 0x10);
}

#[test]
fn toml_integers_are_accepted_for_addr_and_size() {
    let m = parse(
        r#"
        [[regions]]
        name = "data"
        space = 1
        addr = 65536
        size = 0x80000
        "#,
    );
    assert_eq!(m.regions[0].space, 1);
    assert_eq!(m.regions[0].addr, 0x1_0000);
    assert_eq!(m.regions[0].size, 0x8_0000);
}

#[test]
fn a_non_hex_string_names_the_accepted_forms() {
    let err = CheckpointManifest::from_toml(
        r#"
        [[regions]]
        name = "r"
        addr = "not-hex"
        size = "10"
        "#,
    )
    .expect_err("non-hex addr must fail");
    let text = err.to_string();
    assert!(text.contains("not-hex"), "{text}");
    assert!(text.contains("hex string like \"0x10000\""), "{text}");
    assert!(text.contains("non-negative integer"), "{text}");
}

#[test]
fn a_negative_integer_is_rejected_by_value() {
    let err = CheckpointManifest::from_toml(
        r#"
        [[regions]]
        name = "r"
        addr = -1
        size = 16
        "#,
    )
    .expect_err("a negative address must fail");
    let text = err.to_string();
    assert!(text.contains("-1"), "{text}");
    assert!(text.contains("non-negative integer"), "{text}");
}

#[test]
fn a_value_of_another_type_is_rejected_naming_the_forms() {
    let err = CheckpointManifest::from_toml(
        r#"
        [[regions]]
        name = "r"
        addr = 1.5
        size = 16
        "#,
    )
    .expect_err("a float is neither form");
    assert!(err.to_string().contains("hex string like"), "{err}");
}

#[test]
fn region_descriptors_carry_the_named_space() {
    let m = parse(
        r#"
        [[regions]]
        name = "child_result"
        space = 2
        addr = "0x100"
        size = "0x10"
        "#,
    );
    let descriptors = m.region_descriptors();
    assert_eq!(descriptors.len(), 1);
    assert_eq!(descriptors[0].name, "child_result");
    assert_eq!(descriptors[0].space, AddressSpaceId::new(2));
    assert_eq!(descriptors[0].addr, 0x100);
    assert_eq!(descriptors[0].size, 0x10);
}

#[test]
fn load_separates_a_missing_file_from_a_malformed_one() {
    let scratch = scratch_labeled("checkpoint_manifest");
    let missing = scratch.join("missing.toml");
    match load(&missing).expect_err("no file") {
        CheckpointManifestError::Read { path, .. } => {
            assert!(path.ends_with("missing.toml"), "{path}");
        }
        other => panic!("expected Read, got {other:?}"),
    }

    let bad = scratch.join("bad.toml");
    std::fs::write(&bad, "[[regions]]\nname = \"r\"\naddr = \"zz\"\nsize = 1\n").expect("write");
    match load(&bad).expect_err("bad hex") {
        CheckpointManifestError::Parse { path, source } => {
            assert!(path.ends_with("bad.toml"), "{path}");
            assert!(source.to_string().contains("zz"), "{source}");
        }
        other => panic!("expected Parse, got {other:?}"),
    }
}

#[test]
fn the_committed_fixture_manifest_loads() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("NPUA80001")
        .join("checkpoint.toml");
    let m = load(&path).expect("the committed manifest parses");
    assert!(m.regions.iter().any(|r| r.name == "code"));
}

#[test]
fn a_negative_space_is_a_parse_error_not_space_zero() {
    let bad = CheckpointManifest::from_toml(
        r#"
        [[regions]]
        name = "r"
        space = -1
        addr = "0x100"
        size = "0x10"
        "#,
    );
    assert!(
        bad.is_err(),
        "a negative space cannot name any address space and must not wrap"
    );
}
