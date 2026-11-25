//! The hypervisor program type selects the LV1 keysets.

use cellgov_ps3_abi::format::sce::{self_version, SELF_PROGRAM_TYPE_LV1};

use super::*;
use crate::keys::KeyVault;

/// A 0x100-byte SCE container whose program identification header at
/// 0xC0 names `program_type` and `version`.
fn container(program_type: u32, version: u64) -> Vec<u8> {
    let mut data = vec![0u8; 0x100];
    data[0..4].copy_from_slice(&0x5343_4500u32.to_be_bytes());
    data[8..10].copy_from_slice(&0x0002u16.to_be_bytes());
    data[12..16].copy_from_slice(&0x20u32.to_be_bytes());
    data[16..24].copy_from_slice(&0x100u64.to_be_bytes());
    data[0x28..0x30].copy_from_slice(&0xC0u64.to_be_bytes());
    data[0xCC..0xD0].copy_from_slice(&program_type.to_be_bytes());
    data[0xD0..0xD8].copy_from_slice(&version.to_be_bytes());
    data
}

fn keyset(section: &str, label: &str, byte: u8) -> String {
    format!(
        "[[{section}]]\n{label}\nerk = \"{}\"\nriv = \"{}\"\n",
        format!("{byte:02x}").repeat(32),
        format!("{byte:02x}").repeat(16)
    )
}

#[test]
fn a_hypervisor_walks_every_lv1_keyset_and_none_of_the_others() {
    let toml = format!(
        "{}{}{}{}{}",
        keyset("app", "label = \"loose\"", 0x61),
        keyset("lv2", "version = \"3.60-3.61\"", 0x71),
        keyset("lv1", "version = \"3.55\"", 0x81),
        keyset("lv1", "version = \"3.60-3.61\"", 0x82),
        keyset("lv1", "label = \"loose\"", 0x83),
    );
    let keys = KeyVault::parse(std::path::Path::new("keys.toml"), toml.as_bytes()).unwrap();
    let hypervisor = container(SELF_PROGRAM_TYPE_LV1, self_version(3, 0x60));
    let err = decrypt_self_to_elf(&hypervisor, &keys).unwrap_err();
    assert!(
        matches!(
            err,
            SceError::NoCandidateOpensEnvelope {
                class: "LV1",
                revision: 2,
                tried: 3
            }
        ),
        "got {err:?}"
    );
}

#[test]
fn a_hypervisor_meeting_a_vault_with_no_lv1_keyset_is_a_named_key_gap() {
    let toml = format!(
        "{}{}",
        keyset("app", "label = \"loose\"", 0x61),
        keyset("lv2", "version = \"4.93\"", 0x71),
    );
    let keys = KeyVault::parse(std::path::Path::new("keys.toml"), toml.as_bytes()).unwrap();
    let err = decrypt_self_to_elf(
        &container(SELF_PROGRAM_TYPE_LV1, self_version(4, 0x93)),
        &keys,
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            SceError::NoLv1Key {
                version: 0x0004_0093_0000_0000
            }
        ),
        "got {err:?}"
    );
    assert!(err.is_key_vault_refusal(), "{err:?}");
    assert!(crate::kernel_decrypt::is_key_gap(&err), "{err:?}");
    assert!(err.to_string().contains("firmware version 4.93"), "{err}");
}
