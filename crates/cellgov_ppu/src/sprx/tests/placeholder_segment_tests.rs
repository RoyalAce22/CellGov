//! Parse and load of a module whose PT_LOAD table holds zero-sized
//! placeholders around its text and data segments.

use cellgov_ps3_abi::format::elf::{ELF64_RELA_SIZE, ELF_PHENTSIZE};

use crate::sprx::test_fixtures::make_test_prx;
use crate::sprx::{load_prx, parse_prx, PrxLoadError};

/// Fixture geometry from [`make_test_prx`].
const PHDR_BASE: usize = 64;
const RELOC_FILE_OFF: usize = 0x3F0;
const DATA_VADDR: u64 = 0x100;
const DATA_END_VADDR: u64 = 0x300;

/// The PT_LOAD index of each content segment in the padded table.
const TEXT_INDEX: usize = 0;
const DATA_INDEX: usize = 3;

/// [`make_test_prx`] with three zero-sized PT_LOADs interleaved among
/// its two real ones, and every relocation index remapped to match.
///
/// The program-header table moves past the relocation segment because
/// the fixture leaves no room for more entries where it sits.
fn with_placeholder_segments(mut buf: Vec<u8>) -> Vec<u8> {
    let old = |i: usize| {
        let at = PHDR_BASE + i * ELF_PHENTSIZE;
        buf[at..at + ELF_PHENTSIZE].to_vec()
    };
    let text = old(0);
    let data = old(1);
    let reloc = old(2);

    // A placeholder occupies an index and nothing else: no file bytes,
    // no memory. Its vaddr is the end of the real data segment, an
    // address no content segment claims.
    let mut empty = data.clone();
    empty[8..16].copy_from_slice(&(RELOC_FILE_OFF as u64).to_be_bytes());
    empty[16..24].copy_from_slice(&DATA_END_VADDR.to_be_bytes());
    empty[32..40].copy_from_slice(&0u64.to_be_bytes());
    empty[40..48].copy_from_slice(&0u64.to_be_bytes());

    let table = [
        text,
        empty.clone(),
        empty.clone(),
        data,
        empty,
        reloc.clone(),
    ];
    let new_phoff = 0x440usize;
    let need = new_phoff + table.len() * ELF_PHENTSIZE;
    if buf.len() < need {
        buf.resize(need, 0);
    }
    for (i, entry) in table.iter().enumerate() {
        let at = new_phoff + i * ELF_PHENTSIZE;
        buf[at..at + ELF_PHENTSIZE].copy_from_slice(entry);
    }
    buf[32..40].copy_from_slice(&(new_phoff as u64).to_be_bytes());
    buf[56..58].copy_from_slice(&(table.len() as u16).to_be_bytes());

    let reloc_bytes = u64::from_be_bytes(
        reloc[32..40]
            .try_into()
            .expect("8-byte p_filesz from the fixture"),
    ) as usize;
    for entry in (0..reloc_bytes / ELF64_RELA_SIZE).map(|i| RELOC_FILE_OFF + i * ELF64_RELA_SIZE) {
        let info = u64::from_be_bytes(
            buf[entry + 8..entry + 16]
                .try_into()
                .expect("8-byte r_info"),
        );
        let remap = |old: u64| match old {
            0 => TEXT_INDEX as u64,
            1 => DATA_INDEX as u64,
            other => other,
        };
        let sym = (info >> 32) & 0xFFFF_FFFF;
        let moved = (sym & !0xFFFF) | (remap((sym >> 8) & 0xFF) << 8) | remap(sym & 0xFF);
        let info = (moved << 32) | (info & 0xFFFF_FFFF);
        buf[entry + 8..entry + 16].copy_from_slice(&info.to_be_bytes());
    }
    buf
}

#[test]
fn placeholder_pt_loads_do_not_shift_which_segment_is_text_or_data() {
    let plain = parse_prx(&make_test_prx()).expect("plain parse");
    let padded = parse_prx(&with_placeholder_segments(make_test_prx())).expect("padded parse");

    // Every PT_LOAD vaddr in program-header order, placeholders
    // included. A relocation names an index into this list.
    assert_eq!(
        padded.segment_vaddrs,
        vec![
            plain.text.vaddr,
            DATA_END_VADDR,
            DATA_END_VADDR,
            DATA_VADDR,
            DATA_END_VADDR,
        ],
    );
    assert_eq!(padded.text.index, TEXT_INDEX);
    assert_eq!(padded.data.index, DATA_INDEX);
    assert_eq!(
        (padded.text.vaddr, padded.text.filesz),
        (plain.text.vaddr, plain.text.filesz)
    );
    assert_eq!(
        (padded.data.vaddr, padded.data.filesz),
        (plain.data.vaddr, plain.data.filesz)
    );
    assert_eq!(padded.toc, plain.toc);
    assert_eq!(
        padded.module_start.map(|o| (o.opd_vaddr, o.code, o.toc)),
        plain.module_start.map(|o| (o.opd_vaddr, o.code, o.toc)),
    );
}

#[test]
fn a_relocation_resolves_against_the_pt_load_its_index_names() {
    let base: u64 = 0x1000_0000;
    let load = |bytes: Vec<u8>| {
        let prx = parse_prx(&bytes).expect("parse");
        let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
        let loaded = load_prx(&prx, &mut mem, base).expect("load");
        // The site at `DATA_VADDR + 0xF0` is the module_start OPD's
        // code word, the target of the fixture's one data relocation.
        let at = (base + DATA_VADDR + 0xF0) as usize;
        let word = u32::from_be_bytes([
            mem.as_bytes()[at],
            mem.as_bytes()[at + 1],
            mem.as_bytes()[at + 2],
            mem.as_bytes()[at + 3],
        ]);
        (loaded.relocs_applied, loaded.module_start, loaded.toc, word)
    };

    let padded = load(with_placeholder_segments(make_test_prx()));
    assert_eq!(padded, load(make_test_prx()));

    // The OPD relocation names PT_LOAD 0 as its value segment and
    // carries addend 0x10. PT_LOAD 0 sits at vaddr 0.
    let (relocs_applied, _, _, word) = padded;
    assert_eq!(relocs_applied, 3);
    assert_eq!(u64::from(word), base + 0x10);
}

/// Point RELA entry `entry`'s target PT_LOAD index at `index`.
fn retarget(buf: &mut [u8], entry: usize, index: u8) {
    let at = RELOC_FILE_OFF + entry * ELF64_RELA_SIZE + 8;
    let info = u64::from_be_bytes(buf[at..at + 8].try_into().expect("8-byte r_info"));
    let moved = (info & !(0xFFu64 << 32)) | (u64::from(index) << 32);
    buf[at..at + 8].copy_from_slice(&moved.to_be_bytes());
}

#[test]
fn a_relocation_targeting_a_zero_sized_pt_load_is_refused() {
    // Entry 2 is the fixture's data-target relocation; aim it at the
    // placeholder in slot 1.
    let mut bytes = with_placeholder_segments(make_test_prx());
    retarget(&mut bytes, 2, 1);

    let prx = parse_prx(&bytes).expect("parse");
    let mut mem = cellgov_mem::GuestMemory::new(0x2000_0000);
    let err = load_prx(&prx, &mut mem, 0x1000_0000).expect_err("placeholder has nothing to patch");
    match err {
        PrxLoadError::RelocOffsetOutOfSegment {
            offset, seg_size, ..
        } => {
            assert_eq!(offset, 0xF0);
            assert_eq!(seg_size, 0);
        }
        other => panic!("expected RelocOffsetOutOfSegment, got {other:?}"),
    }
}
