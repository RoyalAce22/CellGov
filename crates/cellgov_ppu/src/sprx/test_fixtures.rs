//! Shared in-memory PRX fixtures used by both parse and load tests.

use cellgov_ps3_abi::elf::{
    ELF_MAGIC, ET_PRX, NID_MODULE_START, NID_MODULE_STOP, PT_LOAD, PT_PRX_RELOC,
};

use super::{R_PPC64_ADDR16_HA, R_PPC64_ADDR32};

/// Minimal PRX ELF64 fixture.
///
/// Layout: ELF header at 0, three 56-byte program headers at 0x40, text
/// at 0x0F0 (vaddr 0), data at 0x1F0 (vaddr 0x100, holds module_info,
/// export tables, OPDs), relocations at 0x3F0.
pub(crate) fn make_test_prx() -> Vec<u8> {
    let mut buf = vec![0u8; 0x500];

    buf[0..4].copy_from_slice(&ELF_MAGIC);
    buf[4] = 2;
    buf[5] = 2;
    buf[16..18].copy_from_slice(&ET_PRX.to_be_bytes());
    buf[32..40].copy_from_slice(&64u64.to_be_bytes());
    buf[54..56].copy_from_slice(&56u16.to_be_bytes());
    buf[56..58].copy_from_slice(&3u16.to_be_bytes());

    let phdr_base = 64;

    // PT_LOAD[0] text; p_paddr aliases module_info file offset.
    let ph0 = phdr_base;
    buf[ph0..ph0 + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    buf[ph0 + 8..ph0 + 16].copy_from_slice(&0xF0u64.to_be_bytes());
    buf[ph0 + 16..ph0 + 24].copy_from_slice(&0u64.to_be_bytes());
    buf[ph0 + 24..ph0 + 32].copy_from_slice(&0x1F0u64.to_be_bytes());
    buf[ph0 + 32..ph0 + 40].copy_from_slice(&0x100u64.to_be_bytes());
    buf[ph0 + 40..ph0 + 48].copy_from_slice(&0x100u64.to_be_bytes());

    let ph1 = phdr_base + 56;
    buf[ph1..ph1 + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
    buf[ph1 + 8..ph1 + 16].copy_from_slice(&0x1F0u64.to_be_bytes());
    buf[ph1 + 16..ph1 + 24].copy_from_slice(&0x100u64.to_be_bytes());
    buf[ph1 + 24..ph1 + 32].copy_from_slice(&0u64.to_be_bytes());
    buf[ph1 + 32..ph1 + 40].copy_from_slice(&0x200u64.to_be_bytes());
    buf[ph1 + 40..ph1 + 48].copy_from_slice(&0x200u64.to_be_bytes());

    let ph2 = phdr_base + 112;
    buf[ph2..ph2 + 4].copy_from_slice(&PT_PRX_RELOC.to_be_bytes());
    buf[ph2 + 8..ph2 + 16].copy_from_slice(&0x3F0u64.to_be_bytes());
    buf[ph2 + 32..ph2 + 40].copy_from_slice(&72u64.to_be_bytes());

    // Fill text with nops.
    for i in (0x0F0..0x1F0).step_by(4) {
        buf[i..i + 4].copy_from_slice(&0x6000_0000u32.to_be_bytes());
    }

    let mi = 0x1F0;
    buf[mi..mi + 2].copy_from_slice(&0x0006u16.to_be_bytes());
    buf[mi + 2] = 1;
    buf[mi + 3] = 1;
    buf[mi + 4..mi + 11].copy_from_slice(b"testmod");
    buf[mi + 32..mi + 36].copy_from_slice(&0x200u32.to_be_bytes()); // toc
    buf[mi + 36..mi + 40].copy_from_slice(&0x134u32.to_be_bytes()); // exports_start
    buf[mi + 40..mi + 44].copy_from_slice(&0x16Cu32.to_be_bytes()); // exports_end
    buf[mi + 44..mi + 48].copy_from_slice(&0x16Cu32.to_be_bytes()); // imports_start
    buf[mi + 48..mi + 52].copy_from_slice(&0x16Cu32.to_be_bytes()); // imports_end

    // Export region starts at file 0x224 (vaddr 0x134) to leave the
    // 52-byte library_info struct at file [0x1F0, 0x224) intact.
    let exp0 = 0x224;
    buf[exp0] = 0x1C;
    buf[exp0 + 4..exp0 + 6].copy_from_slice(&0x8000u16.to_be_bytes());
    buf[exp0 + 6..exp0 + 8].copy_from_slice(&2u16.to_be_bytes());
    buf[exp0 + 8..exp0 + 10].copy_from_slice(&1u16.to_be_bytes());
    buf[exp0 + 20..exp0 + 24].copy_from_slice(&0x1A0u32.to_be_bytes());
    buf[exp0 + 24..exp0 + 28].copy_from_slice(&0x1B0u32.to_be_bytes());

    let exp1 = exp0 + 28;
    buf[exp1] = 0x1C;
    buf[exp1 + 4..exp1 + 6].copy_from_slice(&0x0001u16.to_be_bytes());
    buf[exp1 + 6..exp1 + 8].copy_from_slice(&3u16.to_be_bytes());
    buf[exp1 + 16..exp1 + 20].copy_from_slice(&0x1C0u32.to_be_bytes());
    buf[exp1 + 20..exp1 + 24].copy_from_slice(&0x1D0u32.to_be_bytes());
    buf[exp1 + 24..exp1 + 28].copy_from_slice(&0x1E0u32.to_be_bytes());

    let nid0 = 0x290;
    buf[nid0..nid0 + 4].copy_from_slice(&NID_MODULE_START.to_be_bytes());
    buf[nid0 + 4..nid0 + 8].copy_from_slice(&NID_MODULE_STOP.to_be_bytes());
    buf[nid0 + 8..nid0 + 12].copy_from_slice(&0xD7F43016u32.to_be_bytes());

    let stub0 = 0x2A0;
    buf[stub0..stub0 + 4].copy_from_slice(&0x1F0u32.to_be_bytes());
    buf[stub0 + 4..stub0 + 8].copy_from_slice(&0x1F8u32.to_be_bytes());

    let opd_base = 0x2E0;
    buf[opd_base..opd_base + 4].copy_from_slice(&0x10u32.to_be_bytes());
    buf[opd_base + 4..opd_base + 8].copy_from_slice(&0x200u32.to_be_bytes());
    buf[opd_base + 8..opd_base + 12].copy_from_slice(&0x20u32.to_be_bytes());
    buf[opd_base + 12..opd_base + 16].copy_from_slice(&0x200u32.to_be_bytes());

    buf[0x2B0..0x2B7].copy_from_slice(b"testlib");

    let nid1 = 0x2C0;
    buf[nid1..nid1 + 4].copy_from_slice(&0xAAAAAAAAu32.to_be_bytes());
    buf[nid1 + 4..nid1 + 8].copy_from_slice(&0xBBBBBBBBu32.to_be_bytes());
    buf[nid1 + 8..nid1 + 12].copy_from_slice(&0xCCCCCCCCu32.to_be_bytes());

    let stub1 = 0x2D0;
    buf[stub1..stub1 + 4].copy_from_slice(&0x40u32.to_be_bytes());
    buf[stub1 + 4..stub1 + 8].copy_from_slice(&0x50u32.to_be_bytes());
    buf[stub1 + 8..stub1 + 12].copy_from_slice(&0x60u32.to_be_bytes());

    // Three RELA entries (24 bytes each) at 0x3F0.
    // [0] ADDR32 text->text at offset 0x50, addend 0x80.
    let rel0 = 0x3F0;
    buf[rel0..rel0 + 8].copy_from_slice(&0x50u64.to_be_bytes());
    let r_info0: u64 = R_PPC64_ADDR32 as u64;
    buf[rel0 + 8..rel0 + 16].copy_from_slice(&r_info0.to_be_bytes());
    buf[rel0 + 16..rel0 + 24].copy_from_slice(&0x80i64.to_be_bytes());

    // [1] ADDR16_HA text->text at offset 0x54, addend 0x200.
    let rel1 = rel0 + 24;
    buf[rel1..rel1 + 8].copy_from_slice(&0x54u64.to_be_bytes());
    let r_info1: u64 = R_PPC64_ADDR16_HA as u64;
    buf[rel1 + 8..rel1 + 16].copy_from_slice(&r_info1.to_be_bytes());
    buf[rel1 + 16..rel1 + 24].copy_from_slice(&0x200i64.to_be_bytes());

    // [2] ADDR32 target=data value=text at data+0xF0 (module_start
    // OPD code field), addend 0x10.
    let rel2 = rel1 + 24;
    buf[rel2..rel2 + 8].copy_from_slice(&0xF0u64.to_be_bytes());
    let r_info2: u64 = (0x0001u64 << 32) | R_PPC64_ADDR32 as u64;
    buf[rel2 + 8..rel2 + 16].copy_from_slice(&r_info2.to_be_bytes());
    buf[rel2 + 16..rel2 + 24].copy_from_slice(&0x10i64.to_be_bytes());

    buf
}
