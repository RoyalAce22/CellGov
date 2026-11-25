use super::hypercall_tests::{li, KERNEL, NOP};
use super::*;
use crate::instruction::encode::encode;
use cellgov_ps3_abi::format::elf::{
    ELF64_E_ENTRY, ELFCLASS64, ELFDATA2MSB, ELF_EI_CLASS, ELF_EI_DATA, ELF_E_MACHINE_OFFSET,
    ELF_HEADER_SIZE, ELF_MAGIC, ELF_PHENTSIZE, ELF_PHENTSIZE_OFFSET, ELF_PHNUM_OFFSET,
    ELF_PHOFF_OFFSET, EM_PPC64, PF_R, PF_W, PF_X, PHDR_P_FILESZ_OFFSET, PHDR_P_MEMSZ_OFFSET,
    PHDR_P_OFFSET_OFFSET, PHDR_P_VADDR_OFFSET, PT_LOAD,
};
use cellgov_ps3_abi::hw::ppc_isa::PPC_SC_1;

/// The address where the tests put the code: above the real pages of the
/// interrupt vectors.
const CODE: u64 = KERNEL + 0x1_0000;

/// The address where the tests put a second segment.
const OTHER: u64 = KERNEL + 0x2_0000;

/// One PT_LOAD segment of a test ELF, whose memory size is its file size.
pub(super) struct Segment {
    vaddr: u64,
    flags: u32,
    bytes: Vec<u8>,
}

/// An executable segment at `vaddr` that holds `words`.
pub(super) fn code(vaddr: u64, words: &[u32]) -> Segment {
    Segment {
        vaddr,
        flags: PF_R | PF_X,
        bytes: words.iter().flat_map(|word| word.to_be_bytes()).collect(),
    }
}

/// A writable data segment at `vaddr` that holds `bytes`.
pub(super) fn data_bytes(vaddr: u64, bytes: Vec<u8>) -> Segment {
    Segment {
        vaddr,
        flags: PF_R | PF_W,
        bytes,
    }
}

/// A writable data segment at `vaddr` that holds one doubleword.
fn data(vaddr: u64, doubleword: u64) -> Segment {
    Segment {
        vaddr,
        flags: PF_R | PF_W,
        bytes: doubleword.to_be_bytes().to_vec(),
    }
}

/// An ELF with the entry `entry` and the PT_LOAD segments `segments`.
pub(super) fn elf(entry: u64, segments: &[Segment]) -> Vec<u8> {
    let headers = ELF_HEADER_SIZE + segments.len() * ELF_PHENTSIZE;
    let mut elf = vec![0; headers];
    elf[0..4].copy_from_slice(&ELF_MAGIC);
    elf[ELF_EI_CLASS] = ELFCLASS64;
    elf[ELF_EI_DATA] = ELFDATA2MSB;
    elf[6] = 1;
    let mut put = |at: usize, bytes: &[u8]| elf[at..at + bytes.len()].copy_from_slice(bytes);
    put(ELF_E_MACHINE_OFFSET, &EM_PPC64.to_be_bytes());
    put(ELF64_E_ENTRY, &entry.to_be_bytes());
    put(ELF_PHOFF_OFFSET, &(ELF_HEADER_SIZE as u64).to_be_bytes());
    put(ELF_PHENTSIZE_OFFSET, &(ELF_PHENTSIZE as u16).to_be_bytes());
    put(ELF_PHNUM_OFFSET, &(segments.len() as u16).to_be_bytes());
    let mut offset = headers as u64;
    for (index, segment) in segments.iter().enumerate() {
        let header = ELF_HEADER_SIZE + index * ELF_PHENTSIZE;
        let size = (segment.bytes.len() as u64).to_be_bytes();
        put(header, &PT_LOAD.to_be_bytes());
        put(header + 4, &segment.flags.to_be_bytes());
        put(header + PHDR_P_OFFSET_OFFSET, &offset.to_be_bytes());
        put(header + PHDR_P_VADDR_OFFSET, &segment.vaddr.to_be_bytes());
        put(header + PHDR_P_FILESZ_OFFSET, &size);
        put(header + PHDR_P_MEMSZ_OFFSET, &size);
        offset += segment.bytes.len() as u64;
    }
    for segment in segments {
        elf.extend(&segment.bytes);
    }
    elf
}

/// The ordinal of each site that `scan_hypercalls` finds, by address.
pub(super) fn ordinals(entry: u64, segments: &[Segment]) -> Vec<(u64, Option<u16>)> {
    scan_hypercalls(&elf(entry, segments))
        .expect("the ELF scans")
        .into_iter()
        .map(|site| (site.address, site.ordinal))
        .collect()
}

pub(super) fn word(instruction: PpuInstruction) -> u32 {
    encode(&instruction).expect("the instruction encodes")
}

/// `b` at `from` to `to`.
fn b_to(from: u64, to: u64) -> u32 {
    let offset = i32::try_from(to.wrapping_sub(from).cast_signed()).expect("in reach");
    word(PpuInstruction::B {
        offset,
        aa: false,
        link: false,
    })
}

/// `li r11, 74`, `nop`, `sc 1`: the site is word 2, and word 1 lies
/// between the load and `sc`.
fn window() -> [u32; 3] {
    [li(11, 74), NOP, PPC_SC_1]
}

#[test]
fn a_branch_from_another_segment_is_a_path_to_the_site() {
    let from_other = |number: i16| {
        ordinals(
            0,
            &[
                code(CODE, &window()),
                code(OTHER, &[li(11, number), b_to(OTHER + 4, CODE + 4)]),
            ],
        )
    };
    assert_eq!(from_other(74), [(CODE + 8, Some(74))]);
    assert_eq!(from_other(75), [(CODE + 8, None)]);
}

#[test]
fn a_fall_through_from_the_segment_before_is_a_path_to_the_site() {
    let segments = [code(CODE, &[li(11, 75)]), code(CODE + 4, &[PPC_SC_1])];
    assert_eq!(ordinals(0, &segments), [(CODE + 4, Some(75))]);
}

#[test]
fn two_segments_with_different_words_before_the_site_leave_it_unresolved() {
    let overlapping = |number: i16| {
        ordinals(
            0,
            &[
                code(CODE, &[li(11, 74), PPC_SC_1]),
                code(CODE, &[li(11, number), PPC_SC_1]),
            ],
        )
    };
    assert_eq!(
        overlapping(74),
        [(CODE + 4, Some(74)), (CODE + 4, Some(74))]
    );
    assert_eq!(overlapping(75), [(CODE + 4, None), (CODE + 4, None)]);
}

#[test]
fn a_word_a_data_doubleword_names_after_the_load_leaves_the_site_unresolved() {
    let naming = |named: u64| ordinals(0, &[code(CODE, &window()), data(OTHER, named)]);
    assert_eq!(naming(CODE), [(CODE + 8, Some(74))]);
    assert_eq!(naming(CODE + 4), [(CODE + 8, None)]);
}

/// The code builds the address with `lis`, `ori`, `sldi`, `oris` and
/// `ori`, as the kernel does before an `rfid`
/// [OFW-4.93 lv2_kernel a:0x8000000000000320-0x8000000000000338].
#[test]
fn a_word_the_code_builds_the_address_of_leaves_the_site_unresolved() {
    let building = |low: u16| {
        let mut words = vec![
            word(PpuInstruction::Addis {
                rt: 5,
                ra: 0,
                imm: i16::MIN,
            }),
            word(PpuInstruction::Ori {
                ra: 5,
                rs: 5,
                imm: 0,
            }),
            word(PpuInstruction::Rldicr {
                ra: 5,
                rs: 5,
                sh: 32,
                me: 31,
                rc: false,
            }),
            word(PpuInstruction::Oris {
                ra: 5,
                rs: 5,
                imm: 1,
            }),
            word(PpuInstruction::Ori {
                ra: 5,
                rs: 5,
                imm: low,
            }),
            word(PpuInstruction::Mtctr { rs: 5 }),
            word(PpuInstruction::Bcctr {
                bo: 0b10100,
                bi: 0,
                link: false,
            }),
        ];
        words.extend(window());
        ordinals(0, &[code(CODE, &words)])
    };
    assert_eq!(building(0x1C), [(CODE + 0x24, Some(74))]);
    assert_eq!(building(0x20), [(CODE + 0x24, None)]);
}

/// The relative jump table follows its `bctr`, and each entry is an
/// offset from the table's start
/// [OFW-4.93 lv2_kernel a:0x8000000000019b7c-0x8000000000019ba0].
#[test]
fn a_jump_table_target_after_the_load_leaves_the_site_unresolved() {
    let table = |target: u64| {
        let start = CODE + 4;
        let offset = u32::try_from(target - start).expect("a small offset");
        let mut words = vec![
            word(PpuInstruction::Bcctr {
                bo: 0b10100,
                bi: 0,
                link: false,
            }),
            offset,
            0x0010_0000,
        ];
        words.extend(window());
        ordinals(0, &[code(CODE, &words)])
    };
    assert_eq!(table(CODE + 0xC), [(CODE + 0x14, Some(74))]);
    assert_eq!(table(CODE + 0x10), [(CODE + 0x14, None)]);
}

/// The interrupt vectors lie at real addresses 0x100 up to 0x3000
/// [PPC-Book3 p:31 s:4.2.7]. The walk of a `window` at `base` visits the
/// words at `base + 4` and `base + 8`.
#[test]
fn a_window_word_in_the_interrupt_vector_pages_leaves_the_site_unresolved() {
    let at = |base: u64| ordinals(0, &[code(KERNEL + base, &window())]);
    assert_eq!(at(0xF4), [(KERNEL + 0xFC, Some(74))], "0xF8 and 0xFC");
    assert_eq!(at(0xF8), [(KERNEL + 0x100, None)], "0x100");
    assert_eq!(at(0x2FF8), [(KERNEL + 0x3000, None)], "0x2FFC");
    assert_eq!(
        at(0x2FFC),
        [(KERNEL + 0x3004, Some(74))],
        "0x3000 and 0x3004"
    );
}

#[test]
fn a_site_at_the_elf_entry_is_unresolved() {
    let words = [li(11, 74), PPC_SC_1, li(11, 75), PPC_SC_1];
    assert_eq!(
        ordinals(CODE + 0xC, &[code(CODE, &words)]),
        [(CODE + 4, Some(74)), (CODE + 0xC, None)]
    );
}

#[test]
fn a_data_segment_whose_file_bytes_leave_the_elf_is_refused() {
    let mut truncated = elf(0, &[code(CODE, &window()), data(OTHER, CODE + 4)]);
    truncated.truncate(truncated.len() - 1);
    assert!(matches!(
        scan_hypercalls(&truncated),
        Err(CallerScanError::DataSegmentTruncated { index: 1 })
    ));
}
