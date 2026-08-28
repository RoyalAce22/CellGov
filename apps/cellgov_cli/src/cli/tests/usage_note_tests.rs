use super::{DECRYPTION_CLAIMS, SCE_INPUT_USAGE_NOTE};

#[test]
fn note_describes_the_decrypt_support_this_build_has() {
    if cfg!(feature = "decrypt") {
        assert!(SCE_INPUT_USAGE_NOTE.contains("exdata"));
        assert!(!SCE_INPUT_USAGE_NOTE.contains("no decrypt support"));
    } else {
        assert!(SCE_INPUT_USAGE_NOTE.contains("no decrypt support"));
        assert!(!SCE_INPUT_USAGE_NOTE.contains("exdata"));
    }
}

#[test]
fn note_lines_are_tab_indented_for_splicing_under_a_synopsis() {
    assert!(SCE_INPUT_USAGE_NOTE
        .lines()
        .all(|line| line.starts_with('\t')));
}

#[test]
fn funcs_usage_carries_the_note() {
    assert!(crate::funcs::usage().ends_with(SCE_INPUT_USAGE_NOTE));
}

// `ends_with` never looks at the synopsis line above the note.
#[test]
fn funcs_usage_claims_a_decrypt_path_only_when_the_build_has_one() {
    let usage = crate::funcs::usage();
    let has_decrypt = cfg!(feature = "decrypt");
    for claim in DECRYPTION_CLAIMS {
        assert_eq!(
            usage.contains(claim),
            has_decrypt,
            "funcs usage and the build disagree over '{claim}':\n{usage}"
        );
    }
}
