use super::*;
use crate::cli::parse::{Command, DevCommand};
use cellgov_testkit::scratch::scratch_labeled;

const PUP_355: &str = "334e60a4ef5843a688c1c6aebf0951c3259429233c7a5aca5a24f0edad78a192";

fn args(pup_sha256: &str, fw: &str, output_dir: PathBuf) -> RepositoryCensusArgs {
    RepositoryCensusArgs {
        path: "kernel.elf".into(),
        fw: fw.to_string(),
        pup_sha256: pup_sha256.to_string(),
        output_dir,
        max_steps: 100,
    }
}

#[test]
fn repository_census_accepts_the_vfs_root_and_parses_its_arguments() {
    let argv: Vec<String> = [
        "cellgov",
        "--vfs-root",
        "store/dev_hdd0",
        "dev",
        "repository-census",
        "kernel.elf",
        "--fw",
        "3.55",
        "--pup-sha256",
        PUP_355,
        "--output-dir",
        "archive",
        "--max-steps",
        "4000",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let cli = crate::cli::parse::try_parse(&argv).expect("census invocation parses");
    assert_eq!(crate::cli::parse::global_refusal(&cli), None);
    let Command::Dev(DevCommand::RepositoryCensus(args)) = cli.command else {
        panic!("invocation did not parse as dev repository-census");
    };
    assert_eq!(args.path, PathBuf::from("kernel.elf"));
    assert_eq!(args.fw, "3.55");
    assert_eq!(args.pup_sha256, PUP_355);
    assert_eq!(args.output_dir, PathBuf::from("archive"));
    assert_eq!(args.max_steps, 4000);
}

#[test]
fn the_emitter_refuses_a_pup_missing_from_the_archive_before_booting() {
    let missing = "00".repeat(32);
    assert!(matches!(
        emit(&args(&missing, "3.55", "archive".into()), &[]),
        Err(RepositoryCensusError::Extraction(ExtractionError::UnknownPup { pup_sha256 }))
            if pup_sha256 == missing
    ));
}

#[test]
fn the_emitter_refuses_an_elf_that_is_not_its_pups_committed_kernel_and_writes_nothing() {
    let output = scratch_labeled("repository_census_wrong_kernel");
    let result = emit(
        &args(PUP_355, "3.55", output.as_ref().to_path_buf()),
        b"not a kernel",
    );
    assert!(matches!(
        result,
        Err(RepositoryCensusError::Kernel(CommittedKernelError::Verdict(
            archive::KernelElfError::KernelMismatch { pup_sha256, .. }
        )))
            if pup_sha256 == PUP_355
    ));
    assert!(
        !output.join(REPOSITORY_READ.file()).exists(),
        "a refused kernel writes no rows"
    );
}

#[test]
fn the_emitter_refuses_a_pup_with_no_kernel_row_and_writes_nothing() {
    let table = archive::parse(&archive::KERNEL, crate::lv2_tables::KERNEL_TSV)
        .expect("compiled kernel table");
    let kernels = archive::kernel_rows(&table).expect("compiled kernel rows");
    let pups = crate::lv2_tables::committed_pup_rows().expect("compiled PUP table");
    let pup = pups
        .iter()
        .find(|pup| kernels.iter().all(|row| row.pup_sha256 != pup.pup_sha256))
        .expect("pup.tsv holds a PUP with no kernel row");
    let output = scratch_labeled("repository_census_no_kernel_row");
    let error = emit(
        &args(&pup.pup_sha256, &pup.fw, output.as_ref().to_path_buf()),
        b"not a kernel",
    )
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!(
            "PUP {} has no kernel.tsv row; run lv2-census for it first",
            pup.pup_sha256
        )
    );
    assert!(matches!(
        error,
        RepositoryCensusError::Kernel(CommittedKernelError::Verdict(
            archive::KernelElfError::NoKernelRow { pup_sha256 }
        )) if pup_sha256 == pup.pup_sha256
    ));
    assert!(
        !output.join(REPOSITORY_READ.file()).exists(),
        "a PUP with no kernel row writes no rows"
    );
}

#[test]
fn a_missing_table_loads_as_no_rows() {
    let output = scratch_labeled("repository_census_missing");
    assert_eq!(
        load_existing(&output.join(REPOSITORY_READ.file())).expect("load"),
        Vec::new()
    );
}

fn bi_rgntotal() -> cellgov_ps3_abi::lv1::repository::RepositoryKey {
    cellgov_ps3_abi::lv1::repository::RepositoryKey {
        lpar: 2,
        n1: 0x6269_0000,
        n2: 0x7267_6e74_6f74_616c,
        n3: 0,
        n4: 0,
    }
}

fn boot_stopped_by(stop: KernelRunStop) -> KernelRun {
    KernelRun {
        steps: 100,
        stop,
        repository_reads: vec![cellgov_lv1::RepositoryRead {
            key: bi_rgntotal(),
            values: None,
            unit: cellgov_event::UnitId::new(1),
            pc: 0x8000_0000_0000_ce3c,
        }],
        console: Vec::new(),
        thread0: None,
        thread1: None,
    }
}

#[test]
fn a_boot_cut_off_by_its_step_cap_yields_no_rows() {
    assert!(matches!(
        census_rows(PUP_355, 100, &boot_stopped_by(KernelRunStop::StepCap)),
        Err(RepositoryCensusError::StepCap {
            max_steps: 100,
            steps: 100,
            reads: 1,
            ref threads,
        }) if threads.iter().all(Option::is_none)
    ));
}

#[test]
fn a_boot_stopped_by_a_refused_read_keeps_that_read_as_refused() {
    let stop = KernelRunStop::Refused(HypercallRefusal::RepositoryNode {
        key: bi_rgntotal(),
        unit: cellgov_event::UnitId::new(1),
        pc: 0x8000_0000_0000_ce3c,
    });
    let rows =
        census_rows(PUP_355, 100, &boot_stopped_by(stop)).expect("a refusal is a census end");
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].pup_sha256, PUP_355);
    assert_eq!(rows[0].key, bi_rgntotal());
    assert!(!rows[0].answered, "the refused read is logged as refused");
}

#[test]
fn a_refused_repository_read_names_its_key_as_text() {
    let key = cellgov_ps3_abi::lv1::repository::RepositoryKey {
        lpar: 2,
        n1: 0x6269_0000,
        n2: 0x7267_6e74_6f74_616c,
        n3: 0,
        n4: 0,
    };
    let stop = KernelRunStop::Refused(HypercallRefusal::RepositoryNode {
        key,
        unit: cellgov_event::UnitId::new(1),
        pc: 0x8000_0000_0000_ce3c,
    });
    assert_eq!(
        stop_text(&stop),
        "a refused repository read of lpar 2 bi.rgntotal at pc 0x800000000000ce3c"
    );
}
