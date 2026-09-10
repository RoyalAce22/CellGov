use super::*;

use crate::scratch_dir::{scratch, ScratchDir};

/// The `[firmware]` block every fixture manifest opens with.
fn manifest_header() -> String {
    let mut text = String::from(
        "format_version = 2\n\
         [firmware]\n\
         image_version = \"0x0004008200000000\"\n\
         version = \"4.91\"\n\
         pup_sha256 = \"",
    );
    text.push_str(&manifest::Sha256(manifest::sha256_of(b"pup")).to_hex());
    text.push_str("\"\n");
    text
}

/// A `firmware.toml` that names `files`, beside the module bytes it
/// covers.
///
/// Every entry is a plaintext `.prx`, whose own bytes are its
/// post-decrypt image.
fn mount_with(files: &[(&str, &[u8])]) -> ScratchDir {
    let dir = scratch();
    let mut text = manifest_header();
    for (rel, bytes) in files {
        let path = dir.join(rel);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create the module directory");
        }
        std::fs::write(&path, bytes).expect("write the module");
        text.push_str(&format!(
            "[[files]]\npath = \"{rel}\"\nsha256 = \"{}\"\nrevision = 1\n",
            manifest::Sha256(manifest::sha256_of(bytes)).to_hex()
        ));
    }
    std::fs::write(dir.join(MANIFEST_FILE), text).expect("write firmware.toml");
    dir
}

/// A plaintext PRX: ELF magic is all the verifier reads to route it.
fn prx(tail: &[u8]) -> Vec<u8> {
    let mut out = cellgov_ps3_abi::format::elf::ELF_MAGIC.to_vec();
    out.extend_from_slice(tail);
    out
}

#[test]
fn a_mount_without_a_manifest_names_the_mount() {
    let dir = scratch();
    let err = load_manifest(&dir).expect_err("no firmware.toml");
    assert!(
        matches!(&err, FirmwareVerifyError::NoManifest { dir: named } if named == &*dir),
        "got {err}"
    );
}

#[test]
fn a_manifest_of_an_unread_schema_is_refused_by_version() {
    let dir = scratch();
    std::fs::write(dir.join(MANIFEST_FILE), "format_version = 1\n").expect("write");
    let err = load_manifest(&dir).expect_err("schema this build does not read");
    assert!(
        matches!(err, FirmwareVerifyError::ManifestParse { .. }),
        "got {err}"
    );
    assert!(err.to_string().contains(MANIFEST_FILE), "{err}");
}

/// The entry path aims the read the pass hashes.
#[test]
fn a_manifest_entry_path_that_leaves_the_mount_is_refused() {
    for bad in [
        "../outside.prx",
        "sys/../../outside.prx",
        r"sys\external\liblv2.prx",
        "C:/windows/system32/ntdll.dll",
        "sys//external/liblv2.prx",
        "",
    ] {
        let dir = scratch();
        let mut text = manifest_header();
        // A TOML literal string, so a backslash reaches the gate
        // instead of the TOML escape rules.
        text.push_str(&format!(
            "[[files]]\npath = '{bad}'\nsha256 = \"{}\"\nrevision = 1\n",
            manifest::Sha256(manifest::sha256_of(b"x")).to_hex()
        ));
        std::fs::write(dir.join(MANIFEST_FILE), text).expect("write firmware.toml");

        let err = load_manifest(&dir).expect_err("must be refused");
        assert!(
            matches!(&err, FirmwareVerifyError::UnsafeModulePath { entry, .. } if entry == bad),
            "{bad:?} got {err}"
        );
    }
}

#[test]
fn a_manifest_entry_path_inside_the_mount_is_accepted() {
    let dir = mount_with(&[("sys/external/liblv2.prx", &prx(b"lv2"))]);
    let firmware = load_manifest(&dir).expect("load the manifest");
    assert_eq!(firmware.files.len(), 1);
}

#[cfg(not(feature = "decrypt"))]
#[test]
fn a_build_without_decrypt_refuses_the_pass_by_name() {
    let dir = mount_with(&[("sys/external/liblv2.prx", &prx(b"lv2"))]);
    let err = verify_firmware_tree(&dir, &KeyVault::empty()).expect_err("no decrypt support");
    assert!(
        matches!(err, FirmwareVerifyError::DecryptFeatureDisabled),
        "got {err}"
    );
}

#[cfg(feature = "decrypt")]
mod with_decrypt {
    use super::*;

    #[test]
    fn an_intact_tree_is_clean() {
        let dir = mount_with(&[
            ("sys/external/liblv2.prx", &prx(b"lv2")),
            ("sys/external/libsysmodule.prx", &prx(b"sysmodule")),
        ]);
        let report = verify_firmware_tree(&dir, &KeyVault::empty()).expect("verify the tree");
        assert!(report.is_clean(), "divergences: {:?}", report.divergences);
        assert_eq!(report.matched, 2);
    }

    #[test]
    fn a_rewritten_module_reports_both_hashes() {
        let dir = mount_with(&[("sys/external/liblv2.prx", &prx(b"lv2"))]);
        std::fs::write(dir.join("sys/external/liblv2.prx"), prx(b"tampered")).expect("rewrite");
        let report = verify_firmware_tree(&dir, &KeyVault::empty()).expect("verify the tree");
        let [only] = report.divergences.as_slice() else {
            panic!("expected one divergence, got {:?}", report.divergences);
        };
        assert_eq!(
            only.kind,
            ModuleDivergence::Modified {
                expected: manifest::Sha256(manifest::sha256_of(&prx(b"lv2"))),
                found: manifest::Sha256(manifest::sha256_of(&prx(b"tampered"))),
            }
        );
    }

    #[test]
    fn a_deleted_module_is_missing() {
        let dir = mount_with(&[
            ("sys/external/liblv2.prx", &prx(b"lv2")),
            ("sys/external/libsysmodule.prx", &prx(b"sysmodule")),
        ]);
        std::fs::remove_file(dir.join("sys/external/liblv2.prx")).expect("remove the module");
        let report = verify_firmware_tree(&dir, &KeyVault::empty()).expect("verify the tree");
        assert_eq!(
            report.divergences.first().map(|d| &d.kind),
            Some(&ModuleDivergence::Missing)
        );
        assert_eq!(report.matched, 1);
    }

    #[test]
    fn a_module_that_is_neither_a_container_nor_an_elf_yields_no_image() {
        let dir = mount_with(&[("sys/external/liblv2.prx", b"not a module at all")]);
        let report = verify_firmware_tree(&dir, &KeyVault::empty()).expect("verify the tree");
        let [only] = report.divergences.as_slice() else {
            panic!("expected one divergence, got {:?}", report.divergences);
        };
        assert!(
            matches!(&only.kind, ModuleDivergence::NoImage { reason } if reason.contains("ELF")),
            "got {:?}",
            only.kind
        );
    }

    /// The schema permits a manifest with no entries.
    #[test]
    fn a_manifest_covering_no_module_is_refused_rather_than_passing() {
        let dir = mount_with(&[]);
        let err =
            verify_firmware_tree(&dir, &KeyVault::empty()).expect_err("nothing to verify against");
        assert!(
            matches!(err, FirmwareVerifyError::EmptyManifest { .. }),
            "got {err}"
        );
    }

    /// A refusal the vault raises says nothing about the module it was
    /// reading. A refusal about the container's own shape does.
    #[test]
    fn a_vault_refusal_and_a_container_refusal_are_told_apart() {
        use crate::sce::SceError;

        for vault in [
            SceError::NoAppKey { revision: 1 },
            SceError::NoNpdrmKey { revision: 1 },
            SceError::NoCandidateOpensEnvelope {
                class: "APP",
                revision: 1,
                tried: 3,
            },
            SceError::NoRapForNpdrmTitle {
                content_id: "TEST00000".to_string(),
            },
            SceError::RapPboxNotAPermutation { index: 0 },
        ] {
            assert!(is_vault_gap(&vault), "{vault}");
        }
        for container in [
            SceError::BadMagic { got: 0 },
            SceError::KeyEnvelopePadding,
            SceError::ReconstructedBadMagic { got: 0 },
        ] {
            assert!(!is_vault_gap(&container), "{container}");
        }
    }

    #[test]
    fn a_module_the_vault_cannot_open_stops_the_pass_rather_than_diverging() {
        // A revision-0 SCE container: well formed enough to route to the
        // decrypt, and no vault holds a key for it.
        let wrapped = crate::test_support::build_npdrm_eboot_header(2, "SYNTHETIC-0000000");
        let dir = mount_with(&[("sys/external/liblv2.sprx", &wrapped)]);
        let err = verify_firmware_tree(&dir, &KeyVault::empty())
            .expect_err("an empty vault opens nothing");
        assert!(
            matches!(err, FirmwareVerifyError::ModuleKeyMissing { .. }),
            "got {err}"
        );
        assert!(err.to_string().contains("liblv2.sprx"), "{err}");
    }
}
