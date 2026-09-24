//! Each item the disassembly stream yields, and where it ends.

use super::*;
use crate::funcmap::{FunctionName, FunctionOrigin};

/// `ori r0, r0, 0`: the canonical no-op, which decodes.
const NOP: u32 = 0x6000_0000;
/// Primary opcode 0, which no PPU instruction uses.
const ILLEGAL: u32 = 0x0000_0000;

/// A file whose bytes at `file_offset` are `words`, and the segment
/// mapping them at `vaddr` with `memsz` bytes in all.
fn image(words: &[u32], file_offset: u64, vaddr: u64, memsz: u64) -> (Vec<u8>, LoadSegment) {
    let mut elf = vec![0xEEu8; file_offset as usize];
    for w in words {
        elf.extend_from_slice(&w.to_be_bytes());
    }
    let segment = LoadSegment {
        index: 0,
        file_offset,
        vaddr,
        filesz: 4 * words.len() as u64,
        memsz,
        executable: true,
        writable: false,
        readable: true,
    };
    (elf, segment)
}

#[test]
fn each_word_decodes_or_is_named_undecodable_then_the_segment_ends() {
    let (elf, seg) = image(&[NOP, ILLEGAL], 0x10, 0x1000, 8);
    let items: Vec<DisasmItem<'_>> = Disassembly::new(&elf, seg, 0x1000, None).collect();
    assert_eq!(items.len(), 3, "{items:?}");
    assert!(
        matches!(
            items[0],
            DisasmItem::Instruction {
                addr: 0x1000,
                raw: NOP,
                ..
            }
        ),
        "{items:?}"
    );
    assert_eq!(
        items[1],
        DisasmItem::Undecodable {
            addr: 0x1004,
            raw: ILLEGAL
        }
    );
    assert_eq!(
        items[2],
        DisasmItem::End(DisasmEnd::SegmentEnd { addr: 0x1008 })
    );
}

#[test]
fn the_stream_starts_mid_segment_and_stops_at_the_zero_fill() {
    let (elf, seg) = image(&[ILLEGAL, NOP], 0, 0x2000, 0x100);
    let items: Vec<DisasmItem<'_>> = Disassembly::new(&elf, seg, 0x2004, None).collect();
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(matches!(
        items[0],
        DisasmItem::Instruction { addr: 0x2004, .. }
    ));
    assert_eq!(
        items[1],
        DisasmItem::End(DisasmEnd::ZeroFill { addr: 0x2008 })
    );
}

#[test]
fn a_function_start_precedes_its_first_word_once() {
    let (elf, seg) = image(&[NOP, NOP, NOP], 0, 0x3000, 12);
    let span = FunctionSpan {
        start: 0x3004,
        end: 0x300C,
        name: FunctionName::Synthetic,
        origin: FunctionOrigin::OpdScan,
    };
    let map = FunctionMap {
        functions: vec![span],
        truncated: false,
    };
    let items: Vec<DisasmItem<'_>> = Disassembly::new(&elf, seg, 0x3000, Some(&map)).collect();
    let shape: Vec<String> = items
        .iter()
        .map(|item| match item {
            DisasmItem::FunctionStart(s) => format!("fn {:x}", s.start),
            DisasmItem::Instruction { addr, .. } => format!("insn {addr:x}"),
            DisasmItem::Undecodable { addr, .. } => format!("word {addr:x}"),
            DisasmItem::End(end) => format!("{end:?}"),
        })
        .collect();
    assert_eq!(
        shape,
        [
            "insn 3000",
            "fn 3004",
            "insn 3004",
            "insn 3008",
            "SegmentEnd { addr: 12300 }",
        ]
    );
}

#[test]
fn the_address_space_ends_the_stream_where_the_next_word_would_wrap() {
    // A segment claiming the last word of the address space; the word
    // after it has no address.
    let top = u64::MAX - 3;
    let (elf, mut seg) = image(&[NOP], 0, top, 4);
    seg.memsz = 4;
    let items: Vec<DisasmItem<'_>> = Disassembly::new(&elf, seg, top, None).collect();
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(matches!(items[0], DisasmItem::Instruction { addr, .. } if addr == top));
    assert_eq!(
        items[1],
        DisasmItem::End(DisasmEnd::AddressOverflow { words: 1 })
    );
}

#[test]
fn a_start_below_the_segment_ends_at_once_and_nothing_follows_the_end() {
    let (elf, seg) = image(&[NOP], 0, 0x4000, 4);
    let mut stream = Disassembly::new(&elf, seg, 0x3FFC, None);
    assert_eq!(
        stream.next(),
        Some(DisasmItem::End(DisasmEnd::SegmentEnd { addr: 0x3FFC }))
    );
    assert_eq!(stream.next(), None);
}

#[test]
fn a_zero_fill_segment_whose_file_offset_lies_past_the_file_does_not_stop_the_code() {
    use crate::loader::{address_source, checked_pt_loads, AddressSource};
    use cellgov_ps3_abi::format::elf::{ELF_MAGIC, EM_PPC64, EV_CURRENT, PT_LOAD};
    // Header, two program headers, then one code word.
    let code_at: u64 = 64 + 2 * 56;
    let mut elf = vec![0u8; code_at as usize];
    elf[0..4].copy_from_slice(&ELF_MAGIC);
    elf[4] = 2; // ELFCLASS64
    elf[5] = 2; // ELFDATA2MSB
    elf[6] = EV_CURRENT;
    elf[18..20].copy_from_slice(&EM_PPC64.to_be_bytes());
    elf[32..40].copy_from_slice(&64u64.to_be_bytes());
    elf[54..56].copy_from_slice(&56u16.to_be_bytes());
    elf[56..58].copy_from_slice(&2u16.to_be_bytes());
    // (p_offset, p_vaddr, p_filesz, p_memsz): the code, then a
    // zero-fill segment whose p_offset names no byte of the file.
    let loads: [(u64, u64, u64, u64); 2] =
        [(code_at, 0x1_0000, 4, 4), (0x10_0000, 0x2_0000, 0, 0x100)];
    for (i, (offset, vaddr, filesz, memsz)) in loads.into_iter().enumerate() {
        let base = 64 + 56 * i;
        elf[base..base + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
        elf[base + 8..base + 16].copy_from_slice(&offset.to_be_bytes());
        elf[base + 16..base + 24].copy_from_slice(&vaddr.to_be_bytes());
        elf[base + 32..base + 40].copy_from_slice(&filesz.to_be_bytes());
        elf[base + 40..base + 48].copy_from_slice(&memsz.to_be_bytes());
    }
    elf.extend_from_slice(&NOP.to_be_bytes());

    let segments = checked_pt_loads(&elf).expect("a zero-filesz segment claims no file bytes");
    let AddressSource::FileBacked { segment, .. } = address_source(&segments, 0x1_0000, 1) else {
        panic!("the code word is file-backed: {segments:?}");
    };
    let items: Vec<DisasmItem<'_>> = Disassembly::new(&elf, segment, 0x1_0000, None).collect();
    assert_eq!(items.len(), 2, "{items:?}");
    assert!(
        matches!(
            items[0],
            DisasmItem::Instruction {
                addr: 0x1_0000,
                raw: NOP,
                ..
            }
        ),
        "{items:?}"
    );
    assert_eq!(
        items[1],
        DisasmItem::End(DisasmEnd::SegmentEnd { addr: 0x1_0004 })
    );
    assert_eq!(
        address_source(&segments, 0x2_0000, 1),
        AddressSource::ZeroFill {
            segment: segments[1]
        }
    );
}

#[test]
fn file_bytes_the_file_does_not_hold_end_the_stream() {
    let (mut elf, seg) = image(&[NOP, NOP], 0, 0x5000, 8);
    elf.truncate(6);
    let items: Vec<DisasmItem<'_>> = Disassembly::new(&elf, seg, 0x5000, None).collect();
    assert_eq!(items.len(), 2, "{items:?}");
    assert_eq!(
        items[1],
        DisasmItem::End(DisasmEnd::SegmentEnd { addr: 0x5004 })
    );
}
