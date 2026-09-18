//! The plaintext program identification header, and the key class
//! its program type selects.

use cellgov_ps3_abi::format::sce::{self_version, SELF_PROGRAM_TYPE_APP, SELF_PROGRAM_TYPE_LV2};

use super::*;

/// A 0x100-byte SCE container whose program identification header
/// sits at `pid_offset`, typed and versioned as given.
fn container(pid_offset: u64, program_type: u32, version: u64) -> Vec<u8> {
    let mut data = vec![0u8; 0x100];
    data[0..4].copy_from_slice(&0x5343_4500u32.to_be_bytes());
    data[8..10].copy_from_slice(&0x0002u16.to_be_bytes());
    data[12..16].copy_from_slice(&0x20u32.to_be_bytes());
    data[16..24].copy_from_slice(&0x100u64.to_be_bytes());
    data[0x28..0x30].copy_from_slice(&pid_offset.to_be_bytes());
    if let Ok(at) = usize::try_from(pid_offset) {
        if at + 0x20 <= data.len() {
            data[at..at + 8].copy_from_slice(&0x1010_0000_0100_0003u64.to_be_bytes());
            data[at + 8..at + 12].copy_from_slice(&0x0100_0002u32.to_be_bytes());
            data[at + 0x0C..at + 0x10].copy_from_slice(&program_type.to_be_bytes());
            data[at + 0x10..at + 0x18].copy_from_slice(&version.to_be_bytes());
        }
    }
    data
}

#[test]
fn the_header_reads_every_field_at_its_offset() {
    let data = container(0xC0, SELF_PROGRAM_TYPE_LV2, self_version(3, 0x55));
    assert_eq!(
        parse_program_identification(&data).unwrap(),
        ProgramIdentification {
            authority_id: 0x1010_0000_0100_0003,
            vendor_id: 0x0100_0002,
            program_type: SELF_PROGRAM_TYPE_LV2,
            version: 0x0003_0055_0000_0000,
        }
    );
    assert_eq!(
        parse_program_authority_id(&data).unwrap(),
        0x1010_0000_0100_0003
    );
}

#[test]
fn a_header_that_does_not_fit_the_buffer_is_refused_where_the_first_word_alone_would_pass() {
    // 0xF0 leaves 0x10 bytes: the authority id fits, the header does not.
    let data = container(0xF0, SELF_PROGRAM_TYPE_APP, 0);
    assert!(parse_program_authority_id(&data).is_ok());
    assert!(matches!(
        parse_program_identification(&data).unwrap_err(),
        SceError::HeaderOffsetOutOfRange {
            what: "program identification header"
        }
    ));
    assert!(matches!(
        parse_program_identification(&container(0x1000, 0, 0)).unwrap_err(),
        SceError::HeaderOffsetOutOfRange { .. }
    ));
    assert!(matches!(
        parse_program_identification(&data[..0x2C]).unwrap_err(),
        SceError::TooSmall {
            what: "SELF extended header",
            ..
        }
    ));
}

#[cfg(feature = "decrypt")]
#[test]
fn the_program_type_selects_the_key_class_the_decrypt_walks() {
    use crate::keys::KeyVault;

    fn keyset(section: &str, label: &str, byte: u8) -> String {
        format!(
            "[[{section}]]\n{label}\nerk = \"{}\"\nriv = \"{}\"\n",
            format!("{byte:02x}").repeat(32),
            format!("{byte:02x}").repeat(16)
        )
    }
    let toml = format!(
        "{}{}{}",
        keyset("app", "label = \"loose\"", 0x61),
        keyset("lv2", "version = \"3.55\"", 0x71),
        keyset("lv2", "version = \"3.60-3.61\"", 0x72),
    );
    let keys = KeyVault::parse(std::path::Path::new("keys.toml"), toml.as_bytes()).unwrap();

    // A kernel walks both LV2 keysets and none of the APP ones.
    let kernel = container(0xC0, SELF_PROGRAM_TYPE_LV2, self_version(3, 0x60));
    let err = decrypt_self_to_elf(&kernel, &keys).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::NoCandidateOpensEnvelope {
                class: "LV2",
                revision: 2,
                tried: 2
            }
        ),
        "got {err:?}"
    );
    // An application walks the one APP candidate and none of the LV2 ones.
    let app = container(0xC0, SELF_PROGRAM_TYPE_APP, self_version(3, 0x60));
    let err = decrypt_self_to_elf(&app, &keys).unwrap_err();
    assert!(
        matches!(err, SceError::KeyEnvelopePadding),
        "one APP candidate answers its own refusal, got {err:?}"
    );
    let app_only = KeyVault::parse(
        std::path::Path::new("app.toml"),
        keyset("app", "label = \"loose\"", 0x61).as_bytes(),
    )
    .unwrap();
    let err = decrypt_self_to_elf(&kernel, &app_only).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::NoLv2Key {
                version: 0x0003_0060_0000_0000
            }
        ),
        "got {err:?}"
    );
    assert!(err.to_string().contains("firmware version 3.60"), "{err}");
}
