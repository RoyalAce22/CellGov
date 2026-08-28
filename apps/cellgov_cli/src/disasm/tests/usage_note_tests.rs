use crate::cli::exit::{DECRYPTION_CLAIMS, SCE_INPUT_USAGE_NOTE};

#[test]
fn usage_carries_the_shared_sce_input_note() {
    assert!(super::usage().ends_with(SCE_INPUT_USAGE_NOTE));
}

// `ends_with` never looks at the synopsis lines above the note.
#[test]
fn usage_claims_a_decrypt_path_only_when_the_build_has_one() {
    let usage = super::usage();
    let has_decrypt = cfg!(feature = "decrypt");
    for claim in DECRYPTION_CLAIMS {
        assert_eq!(
            usage.contains(claim),
            has_decrypt,
            "disasm usage and the build disagree over '{claim}':\n{usage}"
        );
    }
}
