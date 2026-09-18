use super::*;
use crate::cli::parse::{Command, DevCommand};
use cellgov_testkit::scratch::scratch_labeled;

fn kernel(pup_sha256: &str, kernel_elf_sha256: &str, census_sha256: &str) -> KernelRow {
    KernelRow {
        pup_sha256: pup_sha256.to_string(),
        kernel_elf_sha256: kernel_elf_sha256.to_string(),
        table_base: 0x1000,
        entry_width: 8,
        entry_format: "ppc64_descriptor_pointer".to_string(),
        entry_count: 1024,
        discovery_method: "sc_vector_descriptor_array".to_string(),
        confidence: "high".to_string(),
        census_sha256: census_sha256.to_string(),
        subentry_sha256: "00".repeat(32),
    }
}

#[test]
fn lv2_census_parses_all_provenance_and_output_arguments() {
    let argv: Vec<String> = [
        "cellgov",
        "dev",
        "lv2-census",
        "kernel.elf",
        "--fw",
        "3.55",
        "--pup-sha256",
        "334e60a4ef5843a688c1c6aebf0951c3259429233c7a5aca5a24f0edad78a192",
        "--output-dir",
        "archive",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let cli = crate::cli::parse::try_parse(&argv).expect("census invocation parses");
    assert_eq!(crate::cli::parse::global_refusal(&cli), None);
    let Command::Dev(DevCommand::Lv2Census(args)) = cli.command else {
        panic!("invocation did not parse as dev lv2-census");
    };
    assert_eq!(args.path, PathBuf::from("kernel.elf"));
    assert_eq!(args.fw, "3.55");
    assert_eq!(
        args.pup_sha256,
        "334e60a4ef5843a688c1c6aebf0951c3259429233c7a5aca5a24f0edad78a192"
    );
    assert_eq!(args.output_dir, PathBuf::from("archive"));
    assert!(!args.replace_version);
}

#[test]
fn emitter_refuses_a_pup_missing_from_the_archive_before_reading_the_elf() {
    let args = Lv2CensusArgs {
        path: "kernel.elf".into(),
        fw: "3.55".to_string(),
        pup_sha256: "00".repeat(32),
        output_dir: "archive".into(),
        replace_version: false,
    };
    assert!(matches!(
        emit(&args, &[]),
        Err(Lv2CensusError::UnknownPup { pup_sha256 }) if pup_sha256 == "00".repeat(32)
    ));
}

#[test]
fn emitter_refuses_a_firmware_that_disagrees_with_pup_provenance() {
    let pup = compiled_pups().expect("compiled PUP table")[0].clone();
    let args = Lv2CensusArgs {
        path: "kernel.elf".into(),
        fw: "0.00".to_string(),
        pup_sha256: pup.pup_sha256.clone(),
        output_dir: "archive".into(),
        replace_version: false,
    };
    assert!(matches!(
        emit(&args, &[]),
        Err(Lv2CensusError::FirmwareMismatch {
            pup_sha256,
            recorded,
            requested,
        }) if pup_sha256 == pup.pup_sha256 && recorded == pup.fw && requested == "0.00"
    ));
}

#[test]
fn changed_census_requires_an_explicit_version_replacement() {
    let output = scratch_labeled("lv2_census_replace");
    std::fs::create_dir_all(output.join("census")).expect("create census directory");
    let path = output.join("census/fw-3.55.tsv");
    std::fs::write(&path, "old\n").expect("write existing census");
    let mut args = Lv2CensusArgs {
        path: "kernel.elf".into(),
        fw: "3.55".to_string(),
        pup_sha256: "00".repeat(32),
        output_dir: output.as_ref().to_path_buf(),
        replace_version: false,
    };
    assert!(matches!(
        write_all(&args, "new\n", "kernel\n", "stub\n", "subentry\n"),
        Err(Lv2CensusError::CensusConflict { .. })
    ));
    assert_eq!(
        std::fs::read_to_string(&path).expect("read census"),
        "old\n"
    );

    args.replace_version = true;
    write_all(&args, "new\n", "kernel\n", "stub\n", "subentry\n").expect("replace version");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read census"),
        "new\n"
    );
}

#[test]
fn an_existing_archive_without_subentries_is_refused_as_partial() {
    let output = scratch_labeled("lv2_census_partial_subentry");
    std::fs::create_dir_all(output.as_ref()).expect("create output directory");
    std::fs::write(
        output.join(KERNEL.file()),
        archive::kernel_tsv(&[]).expect("render empty kernel table"),
    )
    .expect("write kernel table");
    std::fs::write(
        output.join(STUB.file()),
        archive::stub_tsv(&[]).expect("render empty stub table"),
    )
    .expect("write stub table");

    assert!(matches!(
        load_existing(output.as_ref()),
        Err(Lv2CensusError::ExistingPartial { .. })
    ));
}

#[test]
fn a_matching_version_census_does_not_hide_a_wrong_pup_kernel() {
    let old = kernel("pup-a", "kernel-a", "same-census");
    let replacement = kernel("pup-a", "kernel-b", "same-census");
    let existing = ExistingRows {
        kernels: vec![old],
        ..ExistingRows::default()
    };
    assert!(matches!(
        refuse_extraction_conflict("pup-a", &existing, &replacement, &[], &[], false),
        Err(Lv2CensusError::KernelDigestConflict { pup_sha256, .. }) if pup_sha256 == "pup-a"
    ));
}

#[test]
fn replace_version_cannot_reassign_a_pup_to_another_kernel() {
    let old = kernel("pup-a", "kernel-a", "old-census");
    let replacement = kernel("pup-a", "kernel-b", "new-census");
    let existing = ExistingRows {
        kernels: vec![old],
        ..ExistingRows::default()
    };
    assert!(matches!(
        refuse_extraction_conflict("pup-a", &existing, &replacement, &[], &[], true),
        Err(Lv2CensusError::KernelDigestConflict { pup_sha256, .. }) if pup_sha256 == "pup-a"
    ));
}

#[test]
fn version_replacement_removes_every_variant_and_reports_the_other_rows() {
    let pups = vec![
        PupRow {
            pup_sha256: "pup-a".to_string(),
            fw: "3.56".to_string(),
            size_bytes: 1,
            image_version: "0x0000000000035600".to_string(),
            source_note: "local".to_string(),
            acquired: None,
        },
        PupRow {
            pup_sha256: "pup-b".to_string(),
            fw: "3.56".to_string(),
            size_bytes: 1,
            image_version: "0x0000000000035600".to_string(),
            source_note: "local".to_string(),
            acquired: None,
        },
        PupRow {
            pup_sha256: "pup-c".to_string(),
            fw: "3.60".to_string(),
            size_bytes: 1,
            image_version: "0x0000000000036000".to_string(),
            source_note: "local".to_string(),
            acquired: None,
        },
    ];
    let mut existing = ExistingRows {
        kernels: vec![
            kernel("pup-a", "kernel-a", "census"),
            kernel("pup-b", "kernel-b", "census"),
            kernel("pup-c", "kernel-c", "other"),
        ],
        stubs: vec![StubRow {
            pup_sha256: "pup-b".to_string(),
            descriptor: 1,
            target: 2,
            errno: 0x8001_0003,
            errno_symbol: "CELL_ENOSYS".to_string(),
            references: 1,
            primary: true,
        }],
        subentries: vec![SubentryRow {
            pup_sha256: "pup-b".to_string(),
            ordinal: 621,
            selector_slot: "r3".to_string(),
            packet: 0,
            class: CensusClass::Implemented,
            target: 3,
        }],
    };
    assert_eq!(
        remove_version_rows("3.56", "pup-a", &pups, &mut existing),
        1
    );
    assert_eq!(
        existing
            .kernels
            .iter()
            .map(|row| row.pup_sha256.as_str())
            .collect::<Vec<_>>(),
        ["pup-c"]
    );
    assert!(existing.stubs.is_empty());
    assert!(existing.subentries.is_empty());
}
