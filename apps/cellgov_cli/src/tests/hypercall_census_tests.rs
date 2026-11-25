use super::*;
use crate::cli::parse::{Command, DevCommand};
use cellgov_ps3_abi::format::elf::{
    ELFCLASS64, ELFDATA2MSB, ELF_EI_CLASS, ELF_EI_DATA, ELF_HEADER_SIZE, ELF_MAGIC, ELF_PHENTSIZE,
    ELF_PHENTSIZE_OFFSET, ELF_PHNUM_OFFSET, ELF_PHOFF_OFFSET, PF_X, PHDR_P_FILESZ_OFFSET,
    PHDR_P_MEMSZ_OFFSET, PHDR_P_OFFSET_OFFSET, PHDR_P_VADDR_OFFSET, PT_LOAD,
};
use cellgov_testkit::scratch::scratch_labeled;

const KERNEL: u64 = 0x8000_0000_0000_0000;
const SC_1: u32 = 0x4400_0022;

fn li_r11(value: u16) -> u32 {
    (14 << 26) | (11 << 21) | u32::from(value)
}

/// A kernel-shaped ELF with one executable segment at `KERNEL + 0x100`
/// that holds `words`.
fn kernel_elf(words: &[u32]) -> Vec<u8> {
    const PAYLOAD: usize = ELF_HEADER_SIZE + ELF_PHENTSIZE;
    let size = 4 * words.len() as u64;
    let mut elf = vec![0; PAYLOAD];
    elf[0..4].copy_from_slice(&ELF_MAGIC);
    elf[ELF_EI_CLASS] = ELFCLASS64;
    elf[ELF_EI_DATA] = ELFDATA2MSB;
    elf[6] = 1;
    elf[18..20].copy_from_slice(&21u16.to_be_bytes());
    let mut put = |at: usize, bytes: &[u8]| elf[at..at + bytes.len()].copy_from_slice(bytes);
    put(ELF_PHOFF_OFFSET, &(ELF_HEADER_SIZE as u64).to_be_bytes());
    put(ELF_PHENTSIZE_OFFSET, &(ELF_PHENTSIZE as u16).to_be_bytes());
    put(ELF_PHNUM_OFFSET, &1u16.to_be_bytes());
    put(ELF_HEADER_SIZE, &PT_LOAD.to_be_bytes());
    put(ELF_HEADER_SIZE + 4, &PF_X.to_be_bytes());
    put(
        ELF_HEADER_SIZE + PHDR_P_OFFSET_OFFSET,
        &(PAYLOAD as u64).to_be_bytes(),
    );
    put(
        ELF_HEADER_SIZE + PHDR_P_VADDR_OFFSET,
        &(KERNEL + 0x100).to_be_bytes(),
    );
    put(ELF_HEADER_SIZE + PHDR_P_FILESZ_OFFSET, &size.to_be_bytes());
    put(ELF_HEADER_SIZE + PHDR_P_MEMSZ_OFFSET, &size.to_be_bytes());
    elf.extend(words.iter().flat_map(|w| w.to_be_bytes()));
    elf
}

#[test]
fn hypercall_census_accepts_the_vfs_root_and_parses_its_arguments() {
    let argv: Vec<String> = [
        "cellgov",
        "--vfs-root",
        "store/dev_hdd0",
        "dev",
        "hypercall-census",
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
    let Command::Dev(DevCommand::HypercallCensus(args)) = cli.command else {
        panic!("invocation did not parse as dev hypercall-census");
    };
    assert_eq!(args.path, PathBuf::from("kernel.elf"));
    assert_eq!(args.fw, "3.55");
    assert_eq!(
        args.pup_sha256,
        "334e60a4ef5843a688c1c6aebf0951c3259429233c7a5aca5a24f0edad78a192"
    );
    assert_eq!(args.output_dir, PathBuf::from("archive"));
}

#[test]
fn the_emitter_refuses_a_pup_missing_from_the_archive_before_scanning() {
    let args = HypercallCensusArgs {
        path: "kernel.elf".into(),
        fw: "3.55".to_string(),
        pup_sha256: "00".repeat(32),
        output_dir: "archive".into(),
    };
    assert!(matches!(
        emit(&args, &[]),
        Err(HypercallCensusError::Extraction(ExtractionError::UnknownPup { pup_sha256 }))
            if pup_sha256 == "00".repeat(32)
    ));
}

#[test]
fn the_emitter_refuses_an_elf_that_is_not_its_pups_committed_kernel() {
    let table = archive::parse(&archive::KERNEL, crate::lv2_tables::KERNEL_TSV)
        .expect("compiled kernel table");
    let kernel = archive::kernel_rows(&table)
        .expect("compiled kernel rows")
        .into_iter()
        .next()
        .expect("kernel.tsv holds a kernel");
    let pups = crate::lv2_tables::committed_pup_rows().expect("compiled PUP table");
    let pup = pups
        .iter()
        .find(|row| row.pup_sha256 == kernel.pup_sha256)
        .expect("the kernel's PUP is in pup.tsv");
    let output = scratch_labeled("hypercall_census_wrong_kernel");
    let args = HypercallCensusArgs {
        path: "kernel.elf".into(),
        fw: pup.fw.clone(),
        pup_sha256: pup.pup_sha256.clone(),
        output_dir: output.as_ref().to_path_buf(),
    };
    let elf = kernel_elf(&[li_r11(74), SC_1]);
    assert!(matches!(
        emit(&args, &elf),
        Err(HypercallCensusError::Kernel(CommittedKernelError::KernelMismatch {
            pup_sha256,
            expected,
            ..
        })) if pup_sha256 == kernel.pup_sha256 && expected == kernel.kernel_elf_sha256
    ));
    assert!(
        !output.join(HYPERCALL_SITE.file()).exists(),
        "a refused kernel writes no rows"
    );
}

#[test]
fn a_kernels_sites_group_by_number_at_their_real_addresses() {
    let sites = [
        CallerSite {
            address: KERNEL + 0x20,
            ordinal: Some(74),
        },
        CallerSite {
            address: KERNEL + 0x10,
            ordinal: Some(74),
        },
        CallerSite {
            address: KERNEL + 0x50,
            ordinal: None,
        },
        CallerSite {
            address: KERNEL + 0x30,
            ordinal: None,
        },
    ];
    let census = census_of("pup", &sites);
    assert_eq!(
        census.sites,
        [HypercallSiteRow {
            pup_sha256: "pup".to_string(),
            number: 74,
            sites: vec![0x10, 0x20],
        }]
    );
    assert_eq!(
        census.unresolved,
        [HypercallUnresolvedRow {
            pup_sha256: "pup".to_string(),
            sites: vec![0x30, 0x50],
        }]
    );
    assert_eq!(
        census_of("pup", &[]).unresolved[0].sites,
        Vec::<u64>::new(),
        "a kernel with no site still names itself scanned"
    );
}

#[test]
fn the_emitter_replaces_its_pups_rows_and_keeps_another_pups() {
    let pups = crate::lv2_tables::committed_pup_rows().expect("compiled PUP table");
    let (ours, theirs) = (&pups[0], &pups[1]);
    let output = scratch_labeled("hypercall_census_emit");
    let held = HypercallCensus {
        sites: vec![
            HypercallSiteRow {
                pup_sha256: ours.pup_sha256.clone(),
                number: 9,
                sites: vec![4],
            },
            HypercallSiteRow {
                pup_sha256: theirs.pup_sha256.clone(),
                number: 9,
                sites: vec![8],
            },
        ],
        unresolved: [ours, theirs]
            .iter()
            .map(|pup| HypercallUnresolvedRow {
                pup_sha256: pup.pup_sha256.clone(),
                sites: vec![],
            })
            .collect(),
    };
    write(
        &output.join(HYPERCALL_SITE.file()),
        &lv1_archive::hypercall_site_tsv(&held.sites).expect("render"),
    )
    .expect("write");
    write(
        &output.join(HYPERCALL_UNRESOLVED.file()),
        &lv1_archive::hypercall_unresolved_tsv(&held.unresolved).expect("render"),
    )
    .expect("write");
    let args = HypercallCensusArgs {
        path: "kernel.elf".into(),
        fw: ours.fw.clone(),
        pup_sha256: ours.pup_sha256.clone(),
        output_dir: output.as_ref().to_path_buf(),
    };
    // The `sc` sites at 0x104 and 0x10c get 74, and the site at 0x114
    // gets 91. The `sc` at 0x118 and 0x11c follow another `sc`, so no
    // load reaches them.
    let elf = kernel_elf(&[
        li_r11(74),
        SC_1,
        li_r11(74),
        SC_1,
        li_r11(91),
        SC_1,
        SC_1,
        SC_1,
    ]);
    assert_eq!(
        write_census(&args, &elf).expect("write the census"),
        EmitSummary {
            numbers: 2,
            resolved: 3,
            unresolved: 2,
        }
    );
    let census = load_existing(output.as_ref()).expect("reload");
    let mut sites: Vec<(String, u64, Vec<u64>)> = census
        .sites
        .into_iter()
        .map(|row| (row.pup_sha256, row.number, row.sites))
        .collect();
    sites.sort();
    let mut expected = vec![
        (ours.pup_sha256.clone(), 74, vec![0x104, 0x10c]),
        (ours.pup_sha256.clone(), 91, vec![0x114]),
        (theirs.pup_sha256.clone(), 9, vec![8]),
    ];
    expected.sort();
    assert_eq!(sites, expected);
    let mut unresolved: Vec<(String, Vec<u64>)> = census
        .unresolved
        .into_iter()
        .map(|row| (row.pup_sha256, row.sites))
        .collect();
    unresolved.sort();
    let mut expected: Vec<(String, Vec<u64>)> = vec![
        (ours.pup_sha256.clone(), vec![0x118, 0x11c]),
        (theirs.pup_sha256.clone(), vec![]),
    ];
    expected.sort();
    assert_eq!(unresolved, expected);
}

#[test]
fn an_existing_archive_with_one_hypercall_table_is_refused_as_partial() {
    let output = scratch_labeled("hypercall_census_partial");
    write(
        &output.join(HYPERCALL_SITE.file()),
        &lv1_archive::hypercall_site_tsv(&[]).expect("render"),
    )
    .expect("write");
    assert!(matches!(
        load_existing(output.as_ref()),
        Err(HypercallCensusError::ExistingPartial { .. })
    ));
}
