//! The EBOOT candidate walk: which refusals stop it, which pass to the
//! next candidate, and when a title counts as not installed.

use std::path::PathBuf;

use cellgov_install::keys::{KeyVault, KeyVaultError};
use cellgov_install::npdrm::NpdLicense;
use cellgov_ps3_abi::format::sce::SCE_MAGIC;

use super::*;
use crate::manifest::test_fixtures::TmpDir;
use crate::manifest::{CheckpointTrigger, Distribution, GameSource};

/// A key source answering every image with one fixed result.
struct Vault(Result<KeyVault, KeyVaultError>);

impl KeyVaultSource for Vault {
    fn vault_for(&self, _bytes: &[u8]) -> Result<&KeyVault, &KeyVaultError> {
        self.0.as_ref()
    }
}

fn empty_vault() -> Vault {
    Vault(Ok(KeyVault::empty()))
}

fn title(candidates: &[&str]) -> TitleManifest {
    TitleManifest {
        content_id: "NPAA00001".to_string(),
        short_name: "t".to_string(),
        display_name: "t".to_string(),
        eboot_candidates: candidates.iter().map(|s| s.to_string()).collect(),
        year: 2007,
        developer: "test-developer".to_string(),
        engine: "test-engine".to_string(),
        distribution: Distribution::PsnHdd,
        rap_filename: None,
        bench_max_steps: None,
        system_ver: None,
        checkpoint: CheckpointTrigger::ProcessExit,
        source: GameSource::Hdd,
        rsx_mirror: false,
        rsx_consume: false,
        content: None,
        mounts: Vec::new(),
        matrix: Vec::new(),
    }
}

fn plaintext_elf() -> Vec<u8> {
    let mut elf = ELF_MAGIC.to_vec();
    elf.extend_from_slice(&[0u8; 60]);
    elf
}

fn npd() -> NpdHeaderInfo {
    NpdHeaderInfo {
        content_id: "NPAA00001".to_string(),
        license: NpdLicense::Local,
    }
}

#[test]
fn a_candidate_that_is_neither_self_nor_elf_passes_the_walk_to_the_next() {
    let tmp = TmpDir::new("eboot_walk_not_elf");
    std::fs::write(tmp.path().join("EBOOT.BIN"), b"junk").unwrap();
    std::fs::write(tmp.path().join("EBOOT.elf"), plaintext_elf()).unwrap();
    let (image, path) = title(&["EBOOT.BIN", "EBOOT.elf"])
        .load_eboot(&[tmp.path().to_path_buf()], tmp.path(), &empty_vault())
        .expect("the plaintext candidate opens");
    assert_eq!(path, tmp.path().join("EBOOT.elf"));
    assert_eq!(image.elf, plaintext_elf());
    assert!(
        image.identity.is_none(),
        "a plaintext ELF has no SELF identity"
    );
}

/// A present file that will not load is a broken dump, not an absent
/// one: the suites skip the second and fail on the first.
#[test]
fn a_dump_whose_only_candidate_is_broken_is_not_reported_as_not_installed() {
    let tmp = TmpDir::new("eboot_walk_all_broken");
    std::fs::write(tmp.path().join("EBOOT.BIN"), b"junk").unwrap();
    let err = title(&["EBOOT.BIN"])
        .load_eboot(&[tmp.path().to_path_buf()], tmp.path(), &empty_vault())
        .unwrap_err();
    let EbootLoadError::AllFailed { attempts, .. } = &err else {
        panic!("expected AllFailed, got {err:?}");
    };
    assert_eq!(
        attempts,
        "    EBOOT.BIN: bytes are not a SELF or plaintext ELF"
    );
}

/// The vault answers every SCE-wrapped candidate the same, so a
/// refusal stops the walk before a plaintext candidate can boot in
/// place of the canonical image.
#[test]
fn a_vault_that_did_not_load_stops_the_walk_before_a_plaintext_candidate() {
    let tmp = TmpDir::new("eboot_walk_vault_refused");
    let mut sce = SCE_MAGIC.to_vec();
    sce.extend_from_slice(&[0u8; 0x100]);
    std::fs::write(tmp.path().join("EBOOT.BIN"), sce).unwrap();
    std::fs::write(tmp.path().join("EBOOT.elf"), plaintext_elf()).unwrap();
    let refusing = Vault(Err(KeyVaultError::MissingScepkg));
    let err = title(&["EBOOT.BIN", "EBOOT.elf"])
        .load_eboot(&[tmp.path().to_path_buf()], tmp.path(), &refusing)
        .unwrap_err();
    assert!(
        matches!(&err, EbootLoadError::Vault { path, .. } if *path == tmp.path().join("EBOOT.BIN")),
        "expected Vault naming EBOOT.BIN, got {err:?}"
    );
}

/// An empty vault refuses every SCE candidate the same way: no APP
/// keyset in a decrypt build, the missing feature in the other. Either
/// refusal stops the walk before the plaintext candidate boots.
#[test]
fn a_decrypt_refusal_stops_the_walk_before_a_plaintext_candidate() {
    let tmp = TmpDir::new("eboot_walk_decrypt_stopped");
    let mut sce = SCE_MAGIC.to_vec();
    sce.extend_from_slice(&[0u8; 0x100]);
    std::fs::write(tmp.path().join("EBOOT.BIN"), sce).unwrap();
    std::fs::write(tmp.path().join("EBOOT.elf"), plaintext_elf()).unwrap();
    let err = title(&["EBOOT.BIN", "EBOOT.elf"])
        .load_eboot(&[tmp.path().to_path_buf()], tmp.path(), &empty_vault())
        .unwrap_err();
    let EbootLoadError::Stopped { path, source, .. } = &err else {
        panic!("expected Stopped, got {err:?}");
    };
    assert_eq!(*path, tmp.path().join("EBOOT.BIN"));
    assert!(
        matches!(
            **source,
            SceError::NoAppKey { .. } | SceError::DecryptFeatureDisabled
        ),
        "{source}"
    );
}

#[test]
fn a_refusal_every_candidate_would_share_stops_the_walk() {
    let stopping = [
        SceError::DecryptFeatureDisabled,
        SceError::NoAppKey { revision: 0x0A },
        SceError::RapRead {
            content_id: "NPAA00001".to_string(),
            source: RapReadError::Missing {
                path: PathBuf::from("NPAA00001.rap"),
            },
        },
    ];
    for e in &stopping {
        assert!(stops_the_walk(e), "{e}");
    }
    let per_candidate = [
        SceError::KeyEnvelopePadding,
        SceError::NoRapForNpdrmTitle {
            content_id: "NPAA00001".to_string(),
        },
        SceError::BadMagic { got: 0 },
    ];
    for e in &per_candidate {
        assert!(!stops_the_walk(e), "{e}");
    }
}

/// The manifest names the RAP, so a missing file is a refusal, not the
/// license-3 free-key fallback.
#[test]
fn a_rap_the_manifest_names_is_required() {
    let tmp = TmpDir::new("eboot_rap_required");
    let mut t = title(&["EBOOT.BIN"]);
    t.rap_filename = Some("NPAA00001.rap".to_string());
    let err = t.rap_lookup(tmp.path())(&npd()).unwrap_err();
    let RapReadError::Missing { path } = &err else {
        panic!("expected Missing, got {err:?}");
    };
    assert_eq!(*path, hdd0_exdata_dir(tmp.path()).join("NPAA00001.rap"));
}

#[test]
fn a_title_that_names_no_rap_looks_up_none() {
    let tmp = TmpDir::new("eboot_rap_none");
    assert_eq!(
        title(&["EBOOT.BIN"]).rap_lookup(tmp.path())(&npd()).unwrap(),
        None
    );
}

fn not_found(
    candidates: &[&str],
    probe_errors: Vec<(PathBuf, std::io::Error)>,
    not_regular: Vec<PathBuf>,
) -> ResolveEbootError {
    ResolveEbootError::NotFound {
        searched: vec![PathBuf::from("USRDIR")],
        candidates: candidates.iter().map(|s| s.to_string()).collect(),
        probe_errors,
        not_regular,
    }
}

#[test]
fn a_miss_of_every_candidate_is_an_absence() {
    assert!(not_found(&["EBOOT.BIN", "EBOOT.elf"], Vec::new(), Vec::new()).is_plain_miss());
}

#[test]
fn a_name_taken_by_a_non_regular_file_is_not_an_absence() {
    let e = not_found(
        &["EBOOT.BIN"],
        Vec::new(),
        vec![PathBuf::from("USRDIR/EBOOT.BIN")],
    );
    assert!(!e.is_plain_miss());
}

#[test]
fn a_probe_that_failed_for_a_reason_other_than_not_found_is_not_an_absence() {
    let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
    let e = not_found(
        &["EBOOT.BIN"],
        vec![(PathBuf::from("USRDIR/EBOOT.BIN"), denied)],
        Vec::new(),
    );
    assert!(!e.is_plain_miss());
}

#[test]
fn a_manifest_with_nothing_to_probe_is_not_an_absence() {
    assert!(!not_found(&[], Vec::new(), Vec::new()).is_plain_miss());
}

#[test]
fn a_misconfigured_vfs_root_is_not_an_absence() {
    let e = ResolveEbootError::MisconfiguredVfsRoot {
        vfs_root: PathBuf::from("/"),
        short_name: "t".to_string(),
    };
    assert!(!e.is_plain_miss());
}

#[test]
fn each_variant_carries_the_marker_note_and_die_text_the_suites_key_on() {
    let no_dir = TitleNotInstalled::NoContentDirectory {
        title: "t".to_string(),
        source: Box::new(not_found(&["EBOOT.BIN"], Vec::new(), Vec::new())),
    };
    assert_eq!(no_dir.marker_note(), "no content directory");
    assert!(
        no_dir
            .to_string()
            .starts_with("resolve_eboot for title t: "),
        "got {no_dir}"
    );

    let no_candidate = TitleNotInstalled::NoEbootCandidate {
        title: "t".to_string(),
        usrdir: "USRDIR".to_string(),
        attempts: "    EBOOT.BIN: read failed: not found".to_string(),
    };
    assert_eq!(no_candidate.marker_note(), "no eboot candidate present");
    assert_eq!(
        no_candidate.to_string(),
        "every eboot_candidate for title t failed under USRDIR:\n    EBOOT.BIN: read failed: not found"
    );
}
