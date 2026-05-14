//! `R_PPC64_ADDR16_LO_DS` applier: write `(S + A) & 0xFFFC` into the
//! low 16 bits, preserving the existing low 2 bits (the DS-form XO).

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use cellgov_mem::{GuestMemory, PageSize, Region};
use cellgov_ppu::prx_loader::PrxModuleId;
use cellgov_ppu::sprx::{
    load_prx, ParsedPrx, PrxLoadError, PrxRelocation, PrxSegment, RelocMisalignedKind,
    R_PPC64_ADDR16_LO_DS,
};

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

fn parsed_with_lo_ds(offset: u64, addend: i64, prefill: u16) -> ParsedPrx {
    let mut text = vec![0u8; SEG_SIZE as usize];
    text[offset as usize..offset as usize + 2].copy_from_slice(&prefill.to_be_bytes());
    ParsedPrx {
        name: "test_addr16_lo_ds".to_string(),
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
            rtype: R_PPC64_ADDR16_LO_DS,
            sym: 0,
            addend,
        }],
        module_start: None,
        module_stop: None,
    }
}

fn read_u16(mem: &GuestMemory, addr: u64) -> u16 {
    let bytes = mem.as_bytes();
    let a = addr as usize;
    u16::from_be_bytes([bytes[a], bytes[a + 1]])
}

#[test]
fn lo_ds_aligned_value_passes_through() {
    // Probes both masks: prefill = 0xFFFF (so existing & 0x0003 ==
    // 0x0003), value low-16 = 0x1234 (so value & 0xFFFC == 0x1234,
    // already DS-aligned). Expected: 0x1234 | 0x0003 = 0x1237.
    // Catches missing/swapped mask, value-vs-existing flip, and
    // any "no mask, just overwrite" regression in one assertion.
    let mut mem = fresh_memory();
    let parsed = parsed_with_lo_ds(0x100, 0x1234, 0xFFFF);
    let _ = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(read_u16(&mem, BASE + TEXT_VADDR + 0x100), 0x1237);
}

#[test]
fn lo_ds_preserves_xo_subfield_of_target() {
    // [PPC-Book1 p:11 s:1.7] DS field is at IBM bits 16:29 (14
    // bits), concatenated on the right with 0b00 to form a 16-bit
    // signed quantity. Bits 30:31 of the halfword carry the DS-form
    // XO subfield (the instruction's secondary opcode -- e.g. ld
    // is XO = 0b00, lwa is XO = 0b10, ldu is XO = 0b01).
    // The relocation must keep those bits intact while writing the
    // upper 14 bits from the value.
    let mut mem = fresh_memory();
    // XO = 0b01 (ldu).
    let parsed = parsed_with_lo_ds(0x200, 0x1234, 0xAAA1);
    let _ = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(read_u16(&mem, BASE + TEXT_VADDR + 0x200), 0x1235);
}

#[test]
fn lo_ds_preserves_xo_subfield_for_lwa_pattern() {
    // XO = 0b10 (lwa). Exercises the full 2-bit XO preservation
    // mask, not just the LSB.
    let mut mem = fresh_memory();
    let parsed = parsed_with_lo_ds(0x400, 0x5678, 0xBBB2);
    let _ = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(read_u16(&mem, BASE + TEXT_VADDR + 0x400), 0x567A);
}

#[test]
fn lo_ds_preserves_xo_subfield_when_both_bits_set() {
    // XO = 0b11 is reserved by the DS-form opcode tables
    // ([PPC-Book1 p:194 s:Appendix I] tables 14-15): opcode-58
    // uses XO {0=ld, 1=ldu, 2=lwa}, opcode-62 uses XO {0=std,
    // 1=stdu}. This test exercises the 2-bit preservation mask
    // in full, not a real instruction.
    let mut mem = fresh_memory();
    let parsed = parsed_with_lo_ds(0x600, 0x4444, 0xCCC3);
    let _ = load_prx(&parsed, &mut mem, BASE).expect("load");
    assert_eq!(read_u16(&mem, BASE + TEXT_VADDR + 0x600), 0x4447);
}

#[test]
fn lo_ds_rejects_misaligned_value_rather_than_silently_truncating() {
    // ELFv1 PPC64 ABI requires (S+A) be 4-byte aligned for
    // ADDR16_LO_DS; a misaligned value rejects rather than masking
    // the low two bits.
    let mut mem = fresh_memory();
    let parsed = parsed_with_lo_ds(0x300, 0x1237, 0x0002);
    let err = load_prx(&parsed, &mut mem, BASE).unwrap_err();
    match err {
        PrxLoadError::RelocMisaligned {
            rtype,
            kind,
            value: _,
        } => {
            assert_eq!(rtype, R_PPC64_ADDR16_LO_DS);
            assert_eq!(kind, RelocMisalignedKind::EncodedValue);
        }
        other => panic!("expected RelocMisaligned, got {other:?}"),
    }
    // Atomic-batch contract: a failed load commits no writes, so
    // the patch site reads back the region's zero-init bytes
    // (not the segment-staged prefill).
    assert_eq!(read_u16(&mem, BASE + TEXT_VADDR + 0x300), 0x0000);
}
