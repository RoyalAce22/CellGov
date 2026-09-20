//! The system software a disc ships: found, registered, or refused
//! before the install stages any of the title.

#![cfg_attr(not(feature = "decrypt"), allow(unused_imports, dead_code))]

use std::collections::BTreeMap;

use super::*;
use crate::scratch_dir::scratch;
use crate::store::layout::VersionKey;
use crate::store::record::{InstallRecord, SourceRecord, INSTALL_RECORD_FORMAT_VERSION};
use crate::test_support::{build_iso, build_param_sfo, codes, IsoNode, RecordingReporter};
#[cfg(feature = "decrypt")]
use crate::test_support::{build_pup, build_tar};
use cellgov_ps3_abi::format::pup::{ENTRY_ID_UPDATE_FILES, ENTRY_ID_VERSION_TXT};

/// Placeholder identity: no real title and no installed content.
const TITLE_ID: &str = "TEST00000";
const SHIPPED: &str = "2.76";

#[cfg(feature = "decrypt")]
fn keys() -> KeyVault {
    crate::test_support::synthetic_vault()
}

/// A disc tree whose EBOOT opens with the SCE magic and is no SELF, so
/// an install that reaches the proof faults there; `PS3_UPDATE/` holds
/// `pup` when given.
fn disc(pup: Option<Vec<u8>>) -> Vec<u8> {
    let mut eboot = cellgov_ps3_abi::format::sce::SCE_MAGIC.to_vec();
    eboot.extend_from_slice(b" not a SELF");
    let mut roots = vec![IsoNode::Dir(
        "PS3_GAME",
        vec![
            IsoNode::File(
                "PARAM.SFO",
                build_param_sfo(&[("TITLE_ID", TITLE_ID), ("CATEGORY", "DG")]),
            ),
            IsoNode::Dir("USRDIR", vec![IsoNode::File("EBOOT.BIN", eboot)]),
        ],
    )];
    if let Some(pup) = pup {
        roots.push(IsoNode::Dir(
            "PS3_UPDATE",
            vec![IsoNode::File("PS3UPDAT.PUP", pup)],
        ));
    }
    build_iso(roots)
}

/// A PUP naming `SHIPPED` whose `update_files` TAR carries no dev_flash
/// package: it passes the HMAC gate and the version read, and the
/// firmware installer refuses it by name.
#[cfg(feature = "decrypt")]
fn pup_without_packages() -> Vec<u8> {
    let outer = build_tar(&[("spkg_hdr.tar", b"x")]);
    build_pup(
        &keys(),
        0x0002_0076_0000_0000,
        &[
            (ENTRY_ID_VERSION_TXT, format!("{SHIPPED}\n").as_bytes()),
            (ENTRY_ID_UPDATE_FILES, &outer),
        ],
    )
}

fn entries_of(image: &[u8]) -> Vec<iso::IsoEntry> {
    iso::read_iso(image).expect("the synthetic image parses")
}

fn record_firmware(vfs: &Path, version: &str, pup_sha256: HexSha256) {
    let layout = StoreLayout::new(vfs);
    let artifact = Artifact::Firmware {
        version: VersionKey::new(version).unwrap(),
    };
    let record = InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: artifact.kind(),
            version: version.to_string(),
            store_path: format!("firmware/{version}"),
        },
        source: SourceRecord::local("pup", pup_sha256),
        title: None,
        files: BTreeMap::new(),
        rap: None,
        core_os: None,
    };
    let path = layout.record_path(&artifact);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, record.to_toml().unwrap()).unwrap();
}

type HexSha256 = crate::manifest::Sha256;

#[cfg(feature = "decrypt")]
#[test]
fn a_disc_without_an_update_package_ships_no_firmware() {
    let image = disc(None);
    let out = scratch();
    let shipped = shipped_firmware(
        &entries_of(&image),
        &image,
        &keys(),
        &out.join("vfs"),
        InstallOptions::default(),
    )
    .expect("no package is not a failure");
    assert!(shipped.is_none());
}

#[cfg(feature = "decrypt")]
#[test]
fn a_recorded_shipped_version_is_reused_and_nothing_is_unpacked() {
    let pup = pup_without_packages();
    let image = disc(Some(pup.clone()));
    let out = scratch();
    let vfs = out.join("vfs");
    record_firmware(&vfs, SHIPPED, sha256_of(&pup));

    let reporter = RecordingReporter::default();
    let shipped = shipped_firmware(
        &entries_of(&image),
        &image,
        &keys(),
        &vfs,
        InstallOptions {
            progress: &reporter,
            ..Default::default()
        },
    )
    .expect("a recorded version is accepted")
    .expect("the disc ships a version");
    assert_eq!(shipped.version, SHIPPED);
    assert!(
        matches!(
            shipped.disposition,
            ShippedFirmwareDisposition::AlreadyInstalled { same_pup: true }
        ),
        "{:?}",
        shipped.disposition
    );
    assert!(
        reporter.phases().is_empty(),
        "nothing was unpacked, so no firmware phase ran: {:?}",
        reporter.phases()
    );
    assert!(
        !StoreLayout::new(&vfs).firmware_staging_dir().exists(),
        "the firmware installer never staged"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_recorded_version_from_another_package_is_reused_and_named_as_such() {
    let pup = pup_without_packages();
    let image = disc(Some(pup));
    let out = scratch();
    let vfs = out.join("vfs");
    record_firmware(&vfs, SHIPPED, sha256_of(b"a different PUP"));

    let shipped = shipped_firmware(
        &entries_of(&image),
        &image,
        &keys(),
        &vfs,
        InstallOptions::default(),
    )
    .expect("the version, not the bytes, decides")
    .expect("the disc ships a version");
    assert!(matches!(
        shipped.disposition,
        ShippedFirmwareDisposition::AlreadyInstalled { same_pup: false }
    ));
}

#[cfg(feature = "decrypt")]
#[test]
fn declining_the_shipped_firmware_never_opens_the_package() {
    let mut pup = pup_without_packages();
    // Corrupt the payload region: an opened package would fail its
    // HMAC, so a success proves it was never opened.
    let last = pup.len() - 1;
    pup[last] ^= 0xff;
    let image = disc(Some(pup));
    let out = scratch();
    let shipped = shipped_firmware(
        &entries_of(&image),
        &image,
        &keys(),
        &out.join("vfs"),
        InstallOptions {
            shipped_firmware: false,
            ..Default::default()
        },
    )
    .expect("a declined package is not read");
    assert!(shipped.is_none());
}

#[cfg(feature = "decrypt")]
#[test]
fn a_package_that_fails_its_hmac_is_refused_by_name() {
    let mut pup = pup_without_packages();
    let last = pup.len() - 1;
    pup[last] ^= 0xff;
    let image = disc(Some(pup));
    let out = scratch();
    let err = shipped_firmware(
        &entries_of(&image),
        &image,
        &keys(),
        &out.join("vfs"),
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            GameInstallError::ShippedFirmware {
                source: firmware_install::FirmwareInstallError::Pup(
                    pup::PupError::HmacMismatch { .. }
                ),
            }
        ),
        "{err:?}"
    );
    let rendered = err.to_string();
    assert!(rendered.contains(DISC_UPDATE_PUP), "{rendered}");
    assert!(rendered.contains("--no-firmware"), "{rendered}");
}

#[cfg(feature = "decrypt")]
#[test]
fn a_package_without_a_version_entry_is_refused_before_the_store_is_read() {
    let outer = build_tar(&[("spkg_hdr.tar", b"x")]);
    let pup = build_pup(&keys(), 0, &[(ENTRY_ID_UPDATE_FILES, &outer)]);
    let image = disc(Some(pup));
    let out = scratch();
    let err = shipped_firmware(
        &entries_of(&image),
        &image,
        &keys(),
        &out.join("vfs"),
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            GameInstallError::ShippedFirmware {
                source: firmware_install::FirmwareInstallError::Pup(pup::PupError::NoEntry {
                    entry_id: ENTRY_ID_VERSION_TXT,
                }),
            }
        ),
        "{err:?}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_record_under_the_shipped_version_describing_another_entry_is_refused() {
    let image = disc(Some(pup_without_packages()));
    let out = scratch();
    let vfs = out.join("vfs");
    // The 2.76 record slot holds a firmware record that names 2.80.
    let slot = Artifact::Firmware {
        version: VersionKey::new(SHIPPED).unwrap(),
    };
    let record = InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: slot.kind(),
            version: "2.80".to_string(),
            store_path: "firmware/2.80".to_string(),
        },
        source: SourceRecord::local("pup", sha256_of(b"another PUP")),
        title: None,
        files: BTreeMap::new(),
        rap: None,
        core_os: None,
    };
    let path = StoreLayout::new(&vfs).record_path(&slot);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, record.to_toml().unwrap()).unwrap();

    let reporter = RecordingReporter::default();
    let err = shipped_firmware(
        &entries_of(&image),
        &image,
        &keys(),
        &vfs,
        InstallOptions {
            progress: &reporter,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        matches!(
            &err,
            GameInstallError::ShippedFirmware {
                source: firmware_install::FirmwareInstallError::RecordMismatch { version, .. },
            } if version == SHIPPED
        ),
        "{err:?}"
    );
    assert!(
        reporter.phases().is_empty(),
        "the refusal precedes the firmware installer: {:?}",
        reporter.phases()
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn an_unreadable_record_under_the_shipped_version_is_refused_not_read_as_absent() {
    let image = disc(Some(pup_without_packages()));
    let out = scratch();
    let vfs = out.join("vfs");
    let slot = Artifact::Firmware {
        version: VersionKey::new(SHIPPED).unwrap(),
    };
    let path = StoreLayout::new(&vfs).record_path(&slot);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, "format_version = 1\nthis is not a record\n").unwrap();

    let reporter = RecordingReporter::default();
    let err = shipped_firmware(
        &entries_of(&image),
        &image,
        &keys(),
        &vfs,
        InstallOptions {
            progress: &reporter,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            GameInstallError::ShippedFirmware {
                source: firmware_install::FirmwareInstallError::RecordParse { .. },
            }
        ),
        "{err:?}"
    );
    assert!(
        reporter.phases().is_empty(),
        "an unreadable record is not unpacked over: {:?}",
        reporter.phases()
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn an_unrecorded_shipped_version_runs_the_firmware_installer_under_its_own_phase() {
    let image = disc(Some(pup_without_packages()));
    let out = scratch();
    let reporter = RecordingReporter::default();
    let err = shipped_firmware(
        &entries_of(&image),
        &image,
        &keys(),
        &out.join("vfs"),
        InstallOptions {
            progress: &reporter,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        matches!(
            err,
            GameInstallError::ShippedFirmware {
                source: firmware_install::FirmwareInstallError::NoDevFlashPackages,
            }
        ),
        "{err:?}"
    );
    assert_eq!(reporter.phases(), codes(&[Phase::InstallingFirmware]));
}

#[cfg(feature = "decrypt")]
#[test]
fn the_shipped_firmware_is_settled_before_any_of_the_title_is_staged() {
    let pup = pup_without_packages();
    let image = disc(Some(pup.clone()));
    let out = scratch();
    let vfs = out.join("vfs");

    let reporter = RecordingReporter::default();
    let err = install_iso(
        &image,
        &keys(),
        &vfs,
        InstallOptions {
            progress: &reporter,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, GameInstallError::ShippedFirmware { .. }),
        "{err:?}"
    );
    assert_eq!(
        reporter.phases(),
        codes(&[Phase::Reading, Phase::InstallingFirmware])
    );
    assert!(
        !vfs.join("dev_bdvd").exists(),
        "nothing of the title was staged"
    );

    record_firmware(&vfs, SHIPPED, sha256_of(&pup));
    let reporter = RecordingReporter::default();
    let err = install_iso(
        &image,
        &keys(),
        &vfs,
        InstallOptions {
            progress: &reporter,
            ..Default::default()
        },
    )
    .unwrap_err();
    assert!(
        matches!(err, GameInstallError::DecryptProof(_)),
        "with the version recorded the install goes on to the proof: {err:?}"
    );
    assert_eq!(
        reporter.phases(),
        codes(&[Phase::Reading, Phase::Staging, Phase::Proving])
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn a_directory_where_the_package_would_be_is_refused_not_read_as_no_package() {
    let mut eboot = cellgov_ps3_abi::format::sce::SCE_MAGIC.to_vec();
    eboot.extend_from_slice(b" not a SELF");
    let image = build_iso(vec![
        IsoNode::Dir(
            "PS3_GAME",
            vec![
                IsoNode::File(
                    "PARAM.SFO",
                    build_param_sfo(&[("TITLE_ID", TITLE_ID), ("CATEGORY", "DG")]),
                ),
                IsoNode::Dir("USRDIR", vec![IsoNode::File("EBOOT.BIN", eboot)]),
            ],
        ),
        IsoNode::Dir("PS3_UPDATE", vec![IsoNode::Dir("PS3UPDAT.PUP", Vec::new())]),
    ]);
    let out = scratch();
    let err = shipped_firmware(
        &entries_of(&image),
        &image,
        &keys(),
        &out.join("vfs"),
        InstallOptions::default(),
    )
    .unwrap_err();
    assert!(
        matches!(err, GameInstallError::ShippedFirmwareNotAFile),
        "{err:?}"
    );
    assert!(err.to_string().contains(DISC_UPDATE_PUP), "{err}");
}

#[test]
fn an_installer_refusal_naming_the_declared_version_is_the_recorded_entry_reused() {
    use firmware_install::FirmwareInstallError as Fw;
    let same = settle_installer_refusal(
        SHIPPED,
        Fw::VersionInstalled {
            version: SHIPPED.to_string(),
            pup_sha256: sha256_of(b"pup"),
        },
    )
    .expect("the version is installed, by whichever writer");
    assert!(matches!(
        same,
        ShippedFirmwareDisposition::AlreadyInstalled { same_pup: true }
    ));

    let other = settle_installer_refusal(
        SHIPPED,
        Fw::VersionInstalledFromAnotherPup {
            version: SHIPPED.to_string(),
            installed: sha256_of(b"a"),
            incoming: sha256_of(b"b"),
        },
    )
    .expect("the version is installed, from other bytes");
    assert!(matches!(
        other,
        ShippedFirmwareDisposition::AlreadyInstalled { same_pup: false }
    ));
}

#[test]
fn an_installer_refusal_naming_another_version_is_the_package_at_odds_with_its_tree() {
    use firmware_install::FirmwareInstallError as Fw;
    let err = settle_installer_refusal(
        SHIPPED,
        Fw::VersionInstalled {
            version: "2.80".to_string(),
            pup_sha256: sha256_of(b"pup"),
        },
    )
    .unwrap_err();
    assert!(
        matches!(
            &err,
            GameInstallError::ShippedFirmwareVersionMismatch { declared, extracted }
                if declared == SHIPPED && extracted == "2.80"
        ),
        "{err:?}"
    );
}

#[test]
fn every_other_installer_refusal_stands_as_the_shipped_firmware_refusal() {
    use firmware_install::FirmwareInstallError as Fw;
    let err = settle_installer_refusal(SHIPPED, Fw::NoDevFlashPackages).unwrap_err();
    assert!(
        matches!(
            err,
            GameInstallError::ShippedFirmware {
                source: Fw::NoDevFlashPackages
            }
        ),
        "{err:?}"
    );
}
