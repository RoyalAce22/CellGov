use super::*;
use crate::cli::parse::{Command, DevCommand};
use cellgov_testkit::scratch::scratch_labeled;

/// The library names no flag; the command adds the one that accepts
/// the movement.
#[test]
fn a_moved_re_extraction_names_the_flag_that_accepts_it() {
    let moved = merge_refusal(ExtractionError::ExtractionConflict {
        pup_sha256: "pup-a".to_string(),
    });
    assert_eq!(
        moved.to_string(),
        "PUP pup-a re-extracted different kernel or stub rows; pass --replace-version to accept the movement"
    );
    let other = merge_refusal(ExtractionError::DigestConflict {
        fw: "3.55".to_string(),
    });
    assert_eq!(
        other.to_string(),
        "firmware 3.55 kernel rows disagree on their census digest"
    );
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
        Err(Lv2CensusError::Extraction(ExtractionError::UnknownPup { pup_sha256 }))
            if pup_sha256 == "00".repeat(32)
    ));
}

#[test]
fn emitter_refuses_a_firmware_that_disagrees_with_pup_provenance() {
    let pup = crate::lv2_tables::committed_pup_rows().expect("compiled PUP table")[0].clone();
    let args = Lv2CensusArgs {
        path: "kernel.elf".into(),
        fw: "0.00".to_string(),
        pup_sha256: pup.pup_sha256.clone(),
        output_dir: "archive".into(),
        replace_version: false,
    };
    assert!(matches!(
        emit(&args, &[]),
        Err(Lv2CensusError::Extraction(ExtractionError::FirmwareMismatch {
            pup_sha256,
            recorded,
            requested,
        })) if pup_sha256 == pup.pup_sha256 && recorded == pup.fw && requested == "0.00"
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
        write_all(&args, "new\n", "kernel\n", "stub\n", "subentry\n", "gate\n"),
        Err(Lv2CensusError::CensusConflict { fw, path: named }) if fw == "3.55" && named == path
    ));
    assert_eq!(
        std::fs::read_to_string(&path).expect("read census"),
        "old\n"
    );

    args.replace_version = true;
    write_all(&args, "new\n", "kernel\n", "stub\n", "subentry\n", "gate\n")
        .expect("replace version");
    assert_eq!(
        std::fs::read_to_string(&path).expect("read census"),
        "new\n"
    );
}

#[test]
fn an_existing_archive_without_subentries_is_refused_as_partial() {
    let output = scratch_labeled("lv2_census_partial_subentry");
    std::fs::create_dir_all(output.join("tables")).expect("create table directory");
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
