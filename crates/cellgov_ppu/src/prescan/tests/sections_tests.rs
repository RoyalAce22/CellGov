//! Range merging and anonymous executable-section discovery for the prescan ELF walk.

use super::*;

#[test]
fn merge_ranges_coalesces_overlap() {
    let merged = merge_ranges(vec![(0, 100), (50, 150), (200, 300)]);
    assert_eq!(merged, vec![(0, 150), (200, 300)]);
}

#[test]
fn merge_ranges_adjacent_ranges_fuse() {
    let merged = merge_ranges(vec![(0, 100), (100, 200)]);
    assert_eq!(merged, vec![(0, 200)]);
}

#[test]
fn merge_ranges_empty_input_returns_empty() {
    let merged = merge_ranges(Vec::new());
    assert!(merged.is_empty());
}

#[test]
fn merge_ranges_sorts_by_low_bound() {
    let merged = merge_ranges(vec![(500, 600), (100, 200), (300, 400)]);
    assert_eq!(merged, vec![(100, 200), (300, 400), (500, 600)]);
}

#[test]
fn merge_ranges_drops_empty_and_inverted_silently() {
    let merged = merge_ranges(vec![(100, 100), (200, 150), (300, 400)]);
    assert_eq!(merged, vec![(300, 400)]);
}

#[test]
fn merge_ranges_only_inverted_input_yields_empty() {
    let merged = merge_ranges(vec![(100, 100), (200, 150)]);
    assert!(merged.is_empty());
}

// -- ELF builder for executable_sections_anonymous tests --

struct AnonTestSection {
    sh_name: u32,
    sh_type: u32,
    sh_flags: u64,
    sh_offset: u64,
    sh_size: u64,
}

/// Build a minimal ELF64 BE with the given section headers and
/// `e_shstrndx`.
fn build_section_only_elf(sections: &[AnonTestSection], shstrndx: u16) -> Vec<u8> {
    let header = 64u64;
    let shoff = header;
    let shtable_end = shoff + 64 * sections.len() as u64;
    let total = sections
        .iter()
        .map(|s| s.sh_offset + s.sh_size)
        .max()
        .map(|m| m.max(shtable_end))
        .unwrap_or(shtable_end);
    let mut buf = vec![0u8; total as usize];

    buf[0..4].copy_from_slice(b"\x7fELF");
    buf[4] = 2; // ELFCLASS64
    buf[5] = 2; // ELFDATA2MSB
    buf[6] = 1;
    // e_shoff
    buf[40..48].copy_from_slice(&shoff.to_be_bytes());
    // e_shentsize, e_shnum, e_shstrndx
    buf[58..60].copy_from_slice(&(64u16).to_be_bytes());
    buf[60..62].copy_from_slice(&(sections.len() as u16).to_be_bytes());
    buf[62..64].copy_from_slice(&shstrndx.to_be_bytes());

    for (i, s) in sections.iter().enumerate() {
        let base = shoff as usize + i * 64;
        buf[base..base + 4].copy_from_slice(&s.sh_name.to_be_bytes());
        buf[base + 4..base + 8].copy_from_slice(&s.sh_type.to_be_bytes());
        buf[base + 8..base + 16].copy_from_slice(&s.sh_flags.to_be_bytes());
        buf[base + 24..base + 32].copy_from_slice(&s.sh_offset.to_be_bytes());
        buf[base + 32..base + 40].copy_from_slice(&s.sh_size.to_be_bytes());
    }

    buf
}

/// Write strtab payload bytes at the declared offset.
fn write_strtab(buf: &mut [u8], offset: u64, payload: &[u8]) {
    let off = offset as usize;
    buf[off..off + payload.len()].copy_from_slice(payload);
}

impl AnonTestSection {
    fn exec_progbits(sh_name: u32, sh_offset: u64, sh_size: u64) -> Self {
        Self {
            sh_name,
            sh_type: SHT_PROGBITS,
            sh_flags: SHF_ALLOC | SHF_EXECINSTR,
            sh_offset,
            sh_size,
        }
    }
    fn strtab(sh_name: u32, sh_offset: u64, sh_size: u64) -> Self {
        Self {
            sh_name,
            sh_type: SHT_STRTAB,
            sh_flags: 0,
            sh_offset,
            sh_size,
        }
    }
}

#[test]
fn anonymous_when_shstrndx_is_undef() {
    let elf = build_section_only_elf(&[AnonTestSection::exec_progbits(0, 0x300, 16)], SHN_UNDEF);
    assert!(executable_sections_anonymous(&elf).expect("walk ok"));
}

#[test]
fn anonymous_when_strtab_is_empty() {
    let strtab_off = 0x200;
    let elf = build_section_only_elf(
        &[
            AnonTestSection::exec_progbits(0, 0x300, 16),
            AnonTestSection::strtab(0, strtab_off, 0),
        ],
        1,
    );
    assert!(executable_sections_anonymous(&elf).expect("walk ok"));
}

#[test]
fn anonymous_when_all_exec_section_names_are_empty() {
    let strtab_off = 0x200;
    let strtab_payload: &[u8] = b"\0shstrtab\0";
    let mut elf = build_section_only_elf(
        &[
            AnonTestSection::exec_progbits(0, 0x100, 16),
            AnonTestSection::strtab(1, strtab_off, strtab_payload.len() as u64),
        ],
        1,
    );
    write_strtab(&mut elf, strtab_off, strtab_payload);
    assert!(executable_sections_anonymous(&elf).expect("walk ok"));
}

#[test]
fn named_when_any_exec_section_has_nonempty_name() {
    let strtab_off = 0x200;
    let strtab_payload: &[u8] = b"\0.text\0shstrtab\0";
    let mut elf = build_section_only_elf(
        &[
            AnonTestSection::exec_progbits(1, 0x100, 16),
            AnonTestSection::strtab(7, strtab_off, strtab_payload.len() as u64),
        ],
        1,
    );
    write_strtab(&mut elf, strtab_off, strtab_payload);
    assert!(!executable_sections_anonymous(&elf).expect("walk ok"));
}

#[test]
fn shstrndx_out_of_range_is_malformed() {
    let elf = build_section_only_elf(&[AnonTestSection::exec_progbits(0, 0x100, 16)], 5);
    assert!(matches!(
        executable_sections_anonymous(&elf),
        Err(PrescanError::MalformedSectionTable)
    ));
}

#[test]
fn strtab_section_payload_past_eof_is_malformed() {
    // Truncate after construction so the strtab payload runs
    // past the file end (the builder otherwise grows the buffer).
    let strtab_off = 0x200;
    let mut elf = build_section_only_elf(
        &[
            AnonTestSection::exec_progbits(0, 0x100, 16),
            AnonTestSection::strtab(0, strtab_off, 0x100000),
        ],
        1,
    );
    elf.truncate(0x250); // < strtab_off + sh_size
    assert!(matches!(
        executable_sections_anonymous(&elf),
        Err(PrescanError::MalformedSectionTable)
    ));
}

#[test]
fn shoff_zero_returns_false_so_caller_picks_segment_fallback() {
    let mut elf = vec![0u8; 64];
    elf[0..4].copy_from_slice(b"\x7fELF");
    assert!(!executable_sections_anonymous(&elf).expect("walk ok"));
}

#[test]
fn non_strtab_at_shstrndx_is_anonymous() {
    let elf = build_section_only_elf(
        &[
            AnonTestSection::exec_progbits(0, 0x100, 16),
            // sh_type != SHT_STRTAB (use SHT_SYMTAB = 2)
            AnonTestSection {
                sh_name: 0,
                sh_type: 2,
                sh_flags: 0,
                sh_offset: 0x200,
                sh_size: 16,
            },
        ],
        1,
    );
    assert!(executable_sections_anonymous(&elf).expect("walk ok"));
}
