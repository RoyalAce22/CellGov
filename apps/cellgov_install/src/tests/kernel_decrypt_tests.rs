use std::path::Path;

use cellgov_ps3_abi::format::sce::{self_version, SELF_PROGRAM_TYPE_APP, SELF_PROGRAM_TYPE_LV2};

use super::*;
use crate::keys::{KeyVault, KeyVaultError};
use crate::manifest::Sha256;
use crate::scratch_dir::scratch;
use crate::test_support::synthetic_vault;

const V3_55: u64 = self_version(3, 0x55);
const V3_60: u64 = self_version(3, 0x60);

/// A 0x100-byte SCE container whose program identification header
/// sits at 0xC0 and names `program_type` and `version`. Its envelope
/// is zeros, which no made-up keyset decrypts to zero padding.
fn kernel_container(program_type: u32, version: u64) -> Vec<u8> {
    let mut data = vec![0u8; 0x100];
    data[0..4].copy_from_slice(&0x5343_4500u32.to_be_bytes());
    data[8..10].copy_from_slice(&0x0002u16.to_be_bytes());
    data[12..16].copy_from_slice(&0x20u32.to_be_bytes());
    data[16..24].copy_from_slice(&0x100u64.to_be_bytes());
    data[0x28..0x30].copy_from_slice(&0xC0u64.to_be_bytes());
    data[0xC0 + 0x0C..0xC0 + 0x10].copy_from_slice(&program_type.to_be_bytes());
    data[0xC0 + 0x10..0xC0 + 0x18].copy_from_slice(&version.to_be_bytes());
    data
}

fn record() -> KernelRecord {
    KernelRecord {
        path: "core_os/lv2_kernel.self".to_string(),
        stored_sha256: Sha256([0; 32]),
    }
}

fn store_kernel(entry: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(entry.join("core_os")).unwrap();
    std::fs::write(entry.join("core_os/lv2_kernel.self"), bytes).unwrap();
}

fn lv2_vault(entries: &[(&str, u8)]) -> KeyVault {
    let mut toml = String::new();
    for (version, byte) in entries {
        toml.push_str(&format!(
            "[[lv2]]\nversion = \"{version}\"\nerk = \"{}\"\nriv = \"{}\"\n",
            format!("{byte:02x}").repeat(32),
            format!("{byte:02x}").repeat(16)
        ));
    }
    KeyVault::parse(Path::new("lv2.toml"), toml.as_bytes()).unwrap()
}

#[test]
fn a_kernel_the_entry_does_not_hold_is_unreadable_not_failed() {
    let entry = scratch();
    let coverage = KernelCoverage::of(decrypt_stored_kernel(&entry, &record(), &synthetic_vault()));
    assert_eq!(coverage.label(), "unreadable");
    assert!(
        matches!(&coverage, KernelCoverage::Unreadable { reason } if reason.contains("lv2_kernel.self")),
        "{coverage:?}"
    );
}

#[test]
fn a_vault_with_no_lv2_keyset_is_a_missing_key_naming_the_firmware() {
    let entry = scratch();
    store_kernel(&entry, &kernel_container(SELF_PROGRAM_TYPE_LV2, V3_60));
    let result = decrypt_stored_kernel(&entry, &record(), &KeyVault::empty());
    assert!(
        matches!(
            &result,
            Err(KernelDecryptError::Decrypt {
                version: Some(v),
                source: SceError::NoLv2Key { version },
            }) if *v == V3_60 && *version == V3_60
        ),
        "{result:?}"
    );
    let coverage = KernelCoverage::of(result);
    assert_eq!(
        coverage,
        KernelCoverage::NoKey {
            version: Some(V3_60),
            missing: "an LV2 keyset for firmware 3.60 (the vault holds none)".to_string()
        }
    );
}

#[test]
fn lv2_keysets_that_open_nothing_are_a_missing_key_never_a_failure() {
    let entry = scratch();
    store_kernel(&entry, &kernel_container(SELF_PROGRAM_TYPE_LV2, V3_60));
    // One candidate: its own padding refusal.
    let one = KernelCoverage::of(decrypt_stored_kernel(
        &entry,
        &record(),
        &lv2_vault(&[("3.60-3.61", 0x71)]),
    ));
    assert_eq!(
        one,
        KernelCoverage::NoKey {
            version: Some(V3_60),
            missing: "a keyset for firmware 3.60 (the one candidate in the vault does not open it)"
                .to_string()
        }
    );
    // Two candidates, the labeled-for-3.60 one and another: both tried.
    let two = KernelCoverage::of(decrypt_stored_kernel(
        &entry,
        &record(),
        &lv2_vault(&[("3.60-3.61", 0x71), ("4.20-4.93", 0x73)]),
    ));
    assert_eq!(
        two,
        KernelCoverage::NoKey {
            version: Some(V3_60),
            missing: "one of the 2 LV2 keysets for firmware 3.60 (none in the vault opens it)"
                .to_string()
        }
    );
}

#[test]
fn a_kernel_typed_self_never_consults_the_app_keysets() {
    let entry = scratch();
    // Two APP candidates for key revision 0x0001 (one labeled, one
    // loose) and one LV2 keyset, so each walk answers differently: the
    // LV2 walk alone is the lone candidate's own padding refusal, the
    // APP walk alone tries two, and a walk over both tables tries three.
    let keyset = |section: &str, label: &str, byte: u8| {
        format!(
            "[[{section}]]\n{label}\nerk = \"{}\"\nriv = \"{}\"\n",
            format!("{byte:02x}").repeat(32),
            format!("{byte:02x}").repeat(16)
        )
    };
    let toml = format!(
        "{}{}{}",
        keyset("app", "revision = 0x0001", 0x61),
        keyset("app", "label = \"loose\"", 0x62),
        keyset("lv2", "version = \"3.55\"", 0x91),
    );
    let vault = KeyVault::parse(Path::new("mixed.toml"), toml.as_bytes()).unwrap();

    let mut kernel = kernel_container(SELF_PROGRAM_TYPE_LV2, V3_55);
    kernel[8..10].copy_from_slice(&0x0001u16.to_be_bytes());
    store_kernel(&entry, &kernel);
    let result = decrypt_stored_kernel(&entry, &record(), &vault);
    assert!(
        matches!(
            &result,
            Err(KernelDecryptError::Decrypt {
                source: SceError::KeyEnvelopePadding,
                ..
            })
        ),
        "{result:?}"
    );
    // The same container typed APP, at the same revision, walks the two
    // APP candidates and none of the LV2 ones.
    let mut app = kernel_container(SELF_PROGRAM_TYPE_APP, V3_55);
    app[8..10].copy_from_slice(&0x0001u16.to_be_bytes());
    store_kernel(&entry, &app);
    let result = decrypt_stored_kernel(&entry, &record(), &vault);
    assert!(
        matches!(
            &result,
            Err(KernelDecryptError::Decrypt {
                source: SceError::NoCandidateOpensEnvelope {
                    class: "APP",
                    revision: 1,
                    tried: 2
                },
                ..
            })
        ),
        "{result:?}"
    );
}

#[test]
fn a_container_that_never_reaches_the_envelope_is_a_failure() {
    let entry = scratch();
    let mut data = kernel_container(SELF_PROGRAM_TYPE_LV2, V3_55);
    // Debug flag: refused before any key is consulted.
    data[8..10].copy_from_slice(&0x8002u16.to_be_bytes());
    store_kernel(&entry, &data);
    let coverage = KernelCoverage::of(decrypt_stored_kernel(&entry, &record(), &synthetic_vault()));
    assert_eq!(coverage.label(), "failed");
    assert!(
        matches!(
            &coverage,
            KernelCoverage::Failed {
                version: Some(version),
                reason,
            } if *version == V3_55 && reason.contains("debug/fself")
        ),
        "{coverage:?}"
    );
}

#[test]
fn every_key_gap_variant_is_one_and_the_container_refusals_are_not() {
    assert!(is_key_gap(&SceError::NoLv2Key { version: V3_55 }));
    assert!(is_key_gap(&SceError::NoAppKey { revision: 1 }));
    assert!(is_key_gap(&SceError::KeyEnvelopePadding));
    assert!(is_key_gap(&SceError::AesCbcDecryptFailed));
    assert!(is_key_gap(&SceError::Keys(Box::new(
        KeyVaultError::MissingScepkg
    ))));
    assert!(is_key_gap(&SceError::NoCandidateOpensEnvelope {
        class: "LV2",
        revision: 1,
        tried: 2
    }));
    assert!(!is_key_gap(&SceError::MetadataTooSmall));
    assert!(!is_key_gap(&SceError::DebugSelfUnsupported {
        revision_flags: 0x8000
    }));
    assert!(!is_key_gap(&SceError::NoUsableSection));
}

fn block(kernel: Option<KernelRecord>, omission: Option<&str>) -> CoreOsRecord {
    CoreOsRecord {
        kernel,
        omission: omission.map(str::to_string),
        files: Vec::new(),
    }
}

/// An entry that stores no kernel says why, without a decrypt attempt;
/// one that stores a kernel reports the decrypt's own state.
#[test]
fn an_entry_reports_why_it_stores_no_kernel_or_how_its_kernel_decrypted() {
    let entry = scratch();
    let keys = synthetic_vault();
    assert_eq!(
        entry_kernel_coverage(&entry, None, &keys),
        EntryKernelCoverage::Absent(KernelAbsence::NotRecorded)
    );
    let omitted = block(None, Some("CORE_OS_PACKAGE.pkg names no lv2_kernel.self"));
    assert_eq!(
        entry_kernel_coverage(&entry, Some(&omitted), &keys),
        EntryKernelCoverage::Absent(KernelAbsence::Omitted(Some(
            "CORE_OS_PACKAGE.pkg names no lv2_kernel.self"
        )))
    );
    let silent = block(None, None);
    assert_eq!(
        entry_kernel_coverage(&entry, Some(&silent), &keys),
        EntryKernelCoverage::Absent(KernelAbsence::Omitted(None))
    );
    let stored = block(Some(record()), None);
    assert!(
        matches!(
            entry_kernel_coverage(&entry, Some(&stored), &keys),
            EntryKernelCoverage::Attempted(KernelCoverage::Unreadable { .. })
        ),
        "the entry holds no file at the recorded path"
    );
}

/// Archive versions come first, in archive order, each with its entry
/// when one is installed; installed versions the archive does not name
/// follow in version-key order.
#[test]
fn coverage_rows_follow_the_archive_then_the_unknown_installs() {
    let archive = ["1.00".to_string(), "1.02".to_string()];
    let installed: BTreeMap<String, &str> =
        [("9.99", "late"), ("1.02", "decrypted"), ("0.50", "early")]
            .into_iter()
            .map(|(version, state)| (version.to_string(), state))
            .collect();
    assert_eq!(
        coverage_rows(&archive, installed),
        [
            ("1.00".to_string(), None),
            ("1.02".to_string(), Some("decrypted")),
            ("0.50".to_string(), Some("early")),
            ("9.99".to_string(), Some("late")),
        ]
    );
}
