//! The `[core_os]` block: round trip, the kind gate, and the
//! kernel-or-omission shape.

use super::*;

use crate::game_install::sha256_of;

fn firmware_record(core_os: Option<CoreOsRecord>) -> InstallRecord {
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
        core_os,
    }
}

fn unpacked() -> CoreOsRecord {
    CoreOsRecord {
        files: vec![
            CoreOsFileRecord {
                name: "creserved_0".to_string(),
                size: 0x40000,
            },
            CoreOsFileRecord {
                name: "lv2_kernel.self".to_string(),
                size: 0x183508,
            },
        ],
        kernel: Some(KernelRecord {
            path: "core_os/lv2_kernel.self".to_string(),
            stored_sha256: sha256_of(b"kernel"),
        }),
        omission: None,
    }
}

fn omitted(why: &str) -> CoreOsRecord {
    CoreOsRecord {
        files: Vec::new(),
        kernel: None,
        omission: Some(why.to_string()),
    }
}

fn toml_of(record: &InstallRecord) -> String {
    record.to_toml().expect("serialise")
}

#[test]
fn an_unpacked_kernel_round_trips_with_every_table_entry() {
    let text = toml_of(&firmware_record(Some(unpacked())));
    let kernel_at = text.find("[core_os.kernel]").expect("the kernel block");
    let files_at = text.find("[[core_os.files]]").expect("the file table");
    assert!(
        kernel_at < files_at,
        "the kernel is named before the table it came out of:\n{text}"
    );
    assert!(text.contains("stored_sha256"), "{text}");
    assert!(!text.contains("omission"), "{text}");
    let back = InstallRecord::parse(&text).expect("parse");
    assert_eq!(back.core_os, Some(unpacked()));
}

#[test]
fn an_omitted_kernel_round_trips_with_its_reason_and_no_files() {
    let text = toml_of(&firmware_record(Some(omitted("no package"))));
    assert!(!text.contains("files"), "{text}");
    assert!(!text.contains("kernel"), "{text}");
    let back = InstallRecord::parse(&text).expect("parse");
    assert_eq!(back.core_os, Some(omitted("no package")));
}

#[test]
fn a_record_without_the_block_reads_as_not_unpacked() {
    let text = toml_of(&firmware_record(None));
    assert!(!text.contains("core_os"), "{text}");
    let back = InstallRecord::parse(&text).expect("a record written before the block existed");
    assert!(back.core_os.is_none());
}

#[test]
fn a_title_record_with_the_block_is_refused_by_kind() {
    let mut record = firmware_record(Some(unpacked()));
    record.artifact.kind = ArtifactKind::TitleBase;
    record.artifact.store_path = "dev_hdd0/game/TEST00000".to_string();
    record.title = Some(TitleRecord {
        title_id: "TEST00000".to_string(),
        content_id: "TEST00000".to_string(),
        category: "HG".to_string(),
        title: "T".to_string(),
        distribution: "psn-hdd".to_string(),
        system_ver: None,
        shipped_firmware: None,
    });
    let err = InstallRecord::parse(&toml_of(&record)).unwrap_err();
    assert!(
        matches!(
            err,
            InstallRecordParseError::UnexpectedCoreOsBlock {
                kind: ArtifactKind::TitleBase
            }
        ),
        "{err:?}"
    );
}

#[test]
fn a_block_naming_both_a_kernel_and_an_omission_is_refused() {
    let mut both = unpacked();
    both.omission = Some("and yet".to_string());
    let err = InstallRecord::parse(&toml_of(&firmware_record(Some(both)))).unwrap_err();
    assert!(
        matches!(err, InstallRecordParseError::CoreOsBlockShape),
        "{err:?}"
    );
}

#[test]
fn a_block_naming_neither_a_kernel_nor_an_omission_is_refused() {
    let neither = CoreOsRecord {
        files: unpacked().files,
        kernel: None,
        omission: None,
    };
    let err = InstallRecord::parse(&toml_of(&firmware_record(Some(neither)))).unwrap_err();
    assert!(
        matches!(err, InstallRecordParseError::CoreOsBlockShape),
        "{err:?}"
    );
}

/// `firmware verify` joins the kernel path onto the entry directory.
#[test]
fn a_kernel_path_that_could_leave_the_entry_is_refused() {
    for bad in [
        "../lv2_kernel.self",
        "core_os/../lv2_kernel.self",
        "core_os//lv2_kernel.self",
        "/core_os/lv2_kernel.self",
        "core_os\\lv2_kernel.self",
        "C:/lv2_kernel.self",
        "",
    ] {
        let mut record = unpacked();
        record.kernel.as_mut().expect("unpacked").path = bad.to_string();
        let err = InstallRecord::parse(&toml_of(&firmware_record(Some(record)))).unwrap_err();
        assert!(
            matches!(err, InstallRecordParseError::UnsafeKernelPath { .. }),
            "{bad:?} was accepted: {err:?}"
        );
    }
}

/// `files` defaults when absent, so only the unknown-field gate stops
/// a misspelled table header from reading as an empty table.
#[test]
fn a_misspelled_core_os_key_is_refused_rather_than_read_as_absent() {
    let text = toml_of(&firmware_record(Some(unpacked())));
    let misspelled = text.replace("[[core_os.files]]", "[[core_os.file]]");
    let err = InstallRecord::parse(&misspelled).unwrap_err();
    assert!(matches!(err, InstallRecordParseError::Toml(_)), "{err:?}");

    let misspelled = text.replace("stored_sha256", "sha256");
    let err = InstallRecord::parse(&misspelled).unwrap_err();
    assert!(matches!(err, InstallRecordParseError::Toml(_)), "{err:?}");
}
