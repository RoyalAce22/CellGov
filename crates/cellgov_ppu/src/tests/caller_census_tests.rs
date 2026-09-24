use super::*;
use cellgov_ps3_abi::format::elf::{
    ELFCLASS64, ELFDATA2MSB, ELF_EI_CLASS, ELF_EI_DATA, ELF_HEADER_SIZE, ELF_MAGIC, ELF_PHENTSIZE,
    ELF_PHENTSIZE_OFFSET, ELF_PHNUM_OFFSET, ELF_PHOFF_OFFSET, PF_X, PHDR_P_FILESZ_OFFSET,
    PHDR_P_MEMSZ_OFFSET, PHDR_P_OFFSET_OFFSET, PHDR_P_VADDR_OFFSET, PT_LOAD,
};

fn li(rt: u8, value: i16) -> u32 {
    (14 << 26) | (u32::from(rt) << 21) | u32::from(value as u16)
}

fn branch() -> u32 {
    (18 << 26) | 4
}

fn mr(rt: u8, rs: u8) -> u32 {
    (31 << 26) | (u32::from(rs) << 21) | (u32::from(rt) << 16) | (u32::from(rs) << 11) | (444 << 1)
}

const SC: u32 = 0x4400_0002;

fn elf_with_exec_segment(vaddr: u64, filesz: u64, memsz: u64) -> Vec<u8> {
    const PAYLOAD_OFFSET: usize = ELF_HEADER_SIZE + ELF_PHENTSIZE;

    let mut elf = vec![0; PAYLOAD_OFFSET + usize::try_from(filesz).unwrap()];
    elf[0..4].copy_from_slice(&ELF_MAGIC);
    elf[ELF_EI_CLASS] = ELFCLASS64;
    elf[ELF_EI_DATA] = ELFDATA2MSB;
    elf[6] = 1; // EV_CURRENT
    elf[18..20].copy_from_slice(&21u16.to_be_bytes()); // EM_PPC64
    elf[ELF_PHOFF_OFFSET..ELF_PHOFF_OFFSET + 8]
        .copy_from_slice(&(ELF_HEADER_SIZE as u64).to_be_bytes());
    elf[ELF_PHENTSIZE_OFFSET..ELF_PHENTSIZE_OFFSET + 2]
        .copy_from_slice(&(ELF_PHENTSIZE as u16).to_be_bytes());
    elf[ELF_PHNUM_OFFSET..ELF_PHNUM_OFFSET + 2].copy_from_slice(&1u16.to_be_bytes());
    elf[ELF_HEADER_SIZE..ELF_HEADER_SIZE + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    elf[ELF_HEADER_SIZE + 4..ELF_HEADER_SIZE + 8].copy_from_slice(&PF_X.to_be_bytes());
    elf[ELF_HEADER_SIZE + PHDR_P_OFFSET_OFFSET..ELF_HEADER_SIZE + PHDR_P_OFFSET_OFFSET + 8]
        .copy_from_slice(&(PAYLOAD_OFFSET as u64).to_be_bytes());
    elf[ELF_HEADER_SIZE + PHDR_P_VADDR_OFFSET..ELF_HEADER_SIZE + PHDR_P_VADDR_OFFSET + 8]
        .copy_from_slice(&vaddr.to_be_bytes());
    elf[ELF_HEADER_SIZE + PHDR_P_FILESZ_OFFSET..ELF_HEADER_SIZE + PHDR_P_FILESZ_OFFSET + 8]
        .copy_from_slice(&filesz.to_be_bytes());
    elf[ELF_HEADER_SIZE + PHDR_P_MEMSZ_OFFSET..ELF_HEADER_SIZE + PHDR_P_MEMSZ_OFFSET + 8]
        .copy_from_slice(&memsz.to_be_bytes());
    elf
}

#[test]
fn a_constant_survives_twelve_argument_instructions() {
    let mut words = vec![li(11, 988)];
    words.extend((0u8..12).map(|n| li(3 + n % 7, i16::from(n))));
    words.push(SC);
    assert_eq!(
        scan_words(0x1000, &words),
        [CallerSite {
            address: 0x1034,
            ordinal: Some(988),
        }]
    );
}

#[test]
fn a_thirteenth_intervening_instruction_is_unresolved() {
    let mut words = vec![li(11, 988)];
    words.extend((0u8..=12).map(|n| li(3 + n % 7, i16::from(n))));
    words.push(SC);
    assert_eq!(
        scan_words(0, &words).last().and_then(|site| site.ordinal),
        None
    );
}

#[test]
fn a_branch_or_r11_write_discards_the_candidate() {
    for middle in [branch(), mr(11, 3)] {
        let sites = scan_words(0, &[li(11, 22), middle, SC]);
        assert_eq!(sites[0].ordinal, None);
    }
}

#[test]
fn a_hypercall_is_not_an_lv2_site() {
    let hypercall = SC | (1 << 5);
    assert!(scan_words(0, &[li(11, 22), hypercall]).is_empty());
}

#[test]
fn scanner_validation_matches_loader_segment_bounds() {
    assert!(matches!(
        scan_syscalls(&elf_with_exec_segment(0x1000, 8, 4)),
        Err(CallerScanError::SegmentFileszExceedsMemsz { index: 0, .. })
    ));
    assert!(matches!(
        scan_syscalls(&elf_with_exec_segment(u64::from(u32::MAX), 4, 4)),
        Err(CallerScanError::SegmentOutOfRange { index: 0 })
    ));
}
