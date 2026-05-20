//! `R_PPC64_REL24` applier: patch the 24-bit branch-immediate field of
//! a PowerPC branch instruction with `(S + A - P) & 0x03FFFFFC`. The
//! low two bits (`AA` / `LK`) and the opcode (top 6 bits) stay intact.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use cellgov_mem::{GuestMemory, PageSize, Region};
use cellgov_ppu::prx_loader::PrxModuleId;
use cellgov_ppu::sprx::{
    load_prx, ParsedPrx, PrxLoadError, PrxRelocation, PrxSegment, RelocMisalignedKind,
    R_PPC64_REL24,
};
use cellgov_ps3_abi::ppc_isa::PPC_BL_OPCODE_LK as BL_OPCODE_LK;

const TEXT_VADDR: u64 = 0x0000;
const DATA_VADDR: u64 = 0x1_0000;
const SEG_SIZE: u64 = 0x1_0000;
const BASE: u64 = 0x4000_0000;

fn fresh_memory() -> GuestMemory {
    GuestMemory::from_regions(vec![Region::new(
        0,
        (BASE + 0x10_0000) as usize,
        "main",
        PageSize::Page64K,
    )])
    .expect("memory")
}

fn parsed_with_rel24(
    offset: u64,
    addend: i64,
    target_seg: u8,
    value_seg: u8,
    insn: u32,
) -> ParsedPrx {
    let mut text = vec![0u8; SEG_SIZE as usize];
    text[offset as usize..offset as usize + 4].copy_from_slice(&insn.to_be_bytes());
    ParsedPrx {
        name: "test_rel24".to_string(),
        module_id: PrxModuleId(0),
        toc: 0,
        text: PrxSegment {
            vaddr: TEXT_VADDR,
            filesz: SEG_SIZE,
            memsz: SEG_SIZE,
            data: text,
        },
        data: PrxSegment {
            vaddr: DATA_VADDR,
            filesz: SEG_SIZE,
            memsz: SEG_SIZE,
            data: vec![0u8; SEG_SIZE as usize],
        },
        exports: vec![],
        relocations: vec![PrxRelocation {
            offset,
            rtype: R_PPC64_REL24,
            sym: u32::from(target_seg) | (u32::from(value_seg) << 8),
            addend,
        }],
        module_start: None,
        module_stop: None,
    }
}

fn read_u32(mem: &GuestMemory, addr: u64) -> u32 {
    let bytes = mem.as_bytes();
    let a = addr as usize;
    u32::from_be_bytes([bytes[a], bytes[a + 1], bytes[a + 2], bytes[a + 3]])
}

#[test]
fn rel24_positive_offset_within_text() {
    let mut mem = fresh_memory();
    // Patch site at text+0x100, target at text+0x200, so delta = +0x100.
    let parsed = parsed_with_rel24(0x100, 0x200, 0, 0, BL_OPCODE_LK);
    let _ = load_prx(&parsed, &mut mem, BASE).expect("load");
    let patched = read_u32(&mem, BASE + TEXT_VADDR + 0x100);
    // Opcode + LK preserved; LI field carries the delta.
    assert_eq!(patched & !0x03FF_FFFC, BL_OPCODE_LK & !0x03FF_FFFC);
    assert_eq!(patched & 0x03FF_FFFC, 0x100);
}

#[test]
fn rel24_negative_offset_backward_branch() {
    let mut mem = fresh_memory();
    // Patch site at text+0x300, target at text+0x100 -> delta = -0x200.
    let parsed = parsed_with_rel24(0x300, 0x100, 0, 0, BL_OPCODE_LK);
    let _ = load_prx(&parsed, &mut mem, BASE).expect("load");
    let patched = read_u32(&mem, BASE + TEXT_VADDR + 0x300);
    // Sign-extended negative delta sits in the LI field with 1-bits in
    // the upper 8 bits of the masked range.
    let delta: i32 = -0x200;
    assert_eq!(patched & 0x03FF_FFFC, (delta as u32) & 0x03FF_FFFC);
    // Opcode + LK still intact.
    assert_eq!(patched & 0xFC00_0003, BL_OPCODE_LK & 0xFC00_0003);
}

#[test]
fn rel24_at_max_positive_range_succeeds() {
    let mut mem = fresh_memory();
    // delta = +0x01FF_FFFC (largest representable; +0x0200_0000 overflows).
    let parsed = parsed_with_rel24(0, 0x01FF_FFFC, 0, 0, BL_OPCODE_LK);
    let _ = load_prx(&parsed, &mut mem, BASE).expect("load");
    let patched = read_u32(&mem, BASE + TEXT_VADDR);
    assert_eq!(patched & 0x03FF_FFFC, 0x01FF_FFFC);
}

#[test]
fn rel24_overflow_returns_error_with_exact_delta() {
    let mut mem = fresh_memory();
    // delta = +0x0200_0000 trips the signed-26-bit clamp; the
    // exact-delta assert guards against an inverted-sign or
    // value-instead-of-displacement report.
    let parsed = parsed_with_rel24(0, 0x0200_0000, 0, 0, BL_OPCODE_LK);
    let err = load_prx(&parsed, &mut mem, BASE).unwrap_err();
    assert_eq!(
        err,
        PrxLoadError::RelocOverflow {
            rtype: R_PPC64_REL24,
            delta: 0x0200_0000,
        }
    );
}

#[test]
fn rel24_at_max_negative_range_succeeds() {
    // delta = -0x0200_0000 (smallest representable signed-26-bit
    // displacement). Place the patch at offset 0x100 with addend
    // chosen so value - target = -0x0200_0000.
    let mut mem = fresh_memory();
    let parsed = parsed_with_rel24(0x100, -0x01FF_FF00, 0, 0, BL_OPCODE_LK);
    let _ = load_prx(&parsed, &mut mem, BASE).expect("load");
    let patched = read_u32(&mem, BASE + TEXT_VADDR + 0x100);
    // LI = -2^23 = 0x0080_0000; encoded as 0x0200_0000 in the
    // 26-bit (LI||00) field.
    assert_eq!(patched & 0x03FF_FFFC, 0x0200_0000);
}

#[test]
fn rel24_just_below_min_overflows_with_exact_delta() {
    // delta = -0x0200_0004 -- one step past the inclusive lower
    // bound of the signed-26-bit range, so RelocOverflow fires.
    let mut mem = fresh_memory();
    let parsed = parsed_with_rel24(0x100, -0x01FF_FF04, 0, 0, BL_OPCODE_LK);
    let err = load_prx(&parsed, &mut mem, BASE).unwrap_err();
    assert_eq!(
        err,
        PrxLoadError::RelocOverflow {
            rtype: R_PPC64_REL24,
            delta: -0x0200_0004,
        }
    );
}

#[test]
fn rel24_misaligned_delta_rejected() {
    // value_seg=text, addend has low bit set; delta = addend - offset
    // = 0x101 - 0x100 = 1 has nonzero low bits. The encoded LI||00
    // field can't represent a misaligned target; the applier
    // rejects rather than silently truncating.
    let mut mem = fresh_memory();
    let parsed = parsed_with_rel24(0x100, 0x101, 0, 0, BL_OPCODE_LK);
    let err = load_prx(&parsed, &mut mem, BASE).unwrap_err();
    assert_eq!(
        err,
        PrxLoadError::RelocMisaligned {
            rtype: R_PPC64_REL24,
            kind: RelocMisalignedKind::Displacement,
            value: 1,
        }
    );
}

#[test]
fn rel24_cross_segment_target_text_value_data_resolves_delta() {
    // target_seg=text, value_seg=data. Locks the seg_vaddrs[1]
    // resolution path for REL24 (the existing same-segment tests
    // never exercise value_seg = 1). With text at vaddr 0 and data
    // at 0x1_0000, an offset 0x100 patch site branching to data
    // vaddr 0 with addend 0 yields delta = 0x1_0000 - 0x100 = 0xFF00.
    let mut mem = fresh_memory();
    let parsed = parsed_with_rel24(0x100, 0, 0, 1, BL_OPCODE_LK);
    let _ = load_prx(&parsed, &mut mem, BASE).expect("load");
    let patched = read_u32(&mem, BASE + TEXT_VADDR + 0x100);
    assert_eq!(patched & 0x03FF_FFFC, 0xFF00);
    // Opcode and LK still intact.
    assert_eq!(patched & 0xFC00_0000, 0x4800_0000);
    assert_eq!(patched & 0x1, 1);
}

#[test]
fn rel24_preserves_opcode_and_link_bits() {
    let mut mem = fresh_memory();
    // Patch a `b` (opcode 18, LK=0) at text+0x400 targeting text+0x800.
    let parsed = parsed_with_rel24(
        0x400,
        0x800,
        0,
        0,
        cellgov_ps3_abi::ppc_isa::PPC_B_OPCODE_NO_LK,
    );
    let _ = load_prx(&parsed, &mut mem, BASE).expect("load");
    let patched = read_u32(&mem, BASE + TEXT_VADDR + 0x400);
    assert_eq!(patched & 0xFC00_0000, 0x4800_0000); // opcode 18
    assert_eq!(patched & 0x1, 0); // LK = 0 preserved
    assert_eq!(patched & 0x2, 0); // AA = 0 preserved
}
