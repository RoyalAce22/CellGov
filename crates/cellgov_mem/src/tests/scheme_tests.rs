use super::*;

#[test]
fn tag_version_reads_the_digits_at_the_end_of_the_tag() {
    let cases: &[(&[u8], u32)] = &[
        (b"cellgov-checkpoint-fnv1a/v26", 26),
        (b"cellgov-checkpoint-fnv1a/v1", 1),
        (b"cellgov-checkpoint-fnv1a/v107", 107),
        (b"no-version", 0),
        (b"", 0),
    ];
    assert_eq!(cases.len(), 5);
    for (tag, version) in cases {
        assert_eq!(
            tag_version(tag),
            *version,
            "{}",
            String::from_utf8_lossy(tag)
        );
    }
}

#[test]
fn the_checkpoint_version_is_the_number_after_the_last_slash_v_of_the_tag() {
    let tag = core::str::from_utf8(CHECKPOINT_TAG).expect("an ASCII tag");
    let (_, digits) = tag
        .rsplit_once("/v")
        .expect("the tag ends with /v and a number");
    let version: u32 = digits.parse().expect("the tag ends with a decimal number");
    assert_eq!(CHECKPOINT_VERSION, version);
}
