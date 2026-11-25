//! The `[core_os]` block's hypervisor fields: the shape gate and the
//! path gate the parse applies to them.

use super::*;

use crate::game_install::sha256_of;

fn firmware_record(core_os: CoreOsRecord) -> InstallRecord {
    InstallRecord {
        format_version: INSTALL_RECORD_FORMAT_VERSION,
        artifact: ArtifactRecord {
            kind: ArtifactKind::Firmware,
            version: "4.93".to_string(),
            store_path: "firmware/4.93".to_string(),
        },
        source: SourceRecord::local("pup", sha256_of(b"pup")),
        title: None,
        files: BTreeMap::new(),
        rap: None,
        core_os: Some(core_os),
    }
}

fn with_lv1(lv1: Option<&str>, lv1_omission: Option<&str>) -> CoreOsRecord {
    CoreOsRecord {
        files: Vec::new(),
        kernel: Some(KernelRecord {
            path: "core_os/lv2_kernel.self".to_string(),
            stored_sha256: sha256_of(b"kernel"),
        }),
        omission: None,
        lv1: lv1.map(|path| KernelRecord {
            path: path.to_string(),
            stored_sha256: sha256_of(b"lv1"),
        }),
        lv1_omission: lv1_omission.map(str::to_string),
    }
}

fn parse(core_os: CoreOsRecord) -> Result<InstallRecord, InstallRecordParseError> {
    let text = firmware_record(core_os).to_toml().expect("serialise");
    InstallRecord::parse(&text)
}

#[test]
fn a_stored_hypervisor_and_a_hypervisor_omission_round_trip() {
    for (block, key) in [
        (with_lv1(Some("core_os/lv1.self"), None), "[core_os.lv1]"),
        (
            with_lv1(None, Some("write core_os/lv1.self: denied")),
            "lv1_omission",
        ),
    ] {
        let text = firmware_record(block.clone()).to_toml().expect("serialise");
        assert!(text.contains(key), "{text}");
        let back = InstallRecord::parse(&text).expect("parse");
        assert_eq!(back.core_os, Some(block));
    }
}

#[test]
fn a_block_naming_both_a_hypervisor_and_its_omission_is_refused() {
    let err = parse(with_lv1(Some("core_os/lv1.self"), Some("and yet"))).unwrap_err();
    assert!(
        matches!(err, InstallRecordParseError::CoreOsBlockShape),
        "{err:?}"
    );
}

/// `firmware verify` joins the hypervisor path onto the entry directory.
#[test]
fn a_hypervisor_path_that_could_leave_the_entry_is_refused() {
    for bad in [
        "../lv1.self",
        "core_os/../../lv1.self",
        "/core_os/lv1.self",
        "core_os\\lv1.self",
        "C:/lv1.self",
        "",
    ] {
        let err = parse(with_lv1(Some(bad), None)).unwrap_err();
        assert!(
            matches!(&err, InstallRecordParseError::UnsafeKernelPath { path } if path == bad),
            "{bad:?} was accepted: {err:?}"
        );
    }
}
