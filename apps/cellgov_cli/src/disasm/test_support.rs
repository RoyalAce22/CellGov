//! Synthetic ELF builders shared by `elf::tests` and `stream::tests`.

#![cfg(test)]

use cellgov_ps3_abi::elf::{
    ELFCLASS64, ELFDATA2MSB, ELF_HEADER_SIZE, ELF_PHENTSIZE, EM_PPC64, ET_EXEC, EV_CURRENT, PF_R,
    PF_X, PT_LOAD,
};

/// Synthetic Elf64_Phdr description for `build_elf64_be`.
pub(super) struct SegSpec {
    pub(super) p_type: u32,
    pub(super) p_offset: u64,
    pub(super) p_vaddr: u64,
    pub(super) p_filesz: u64,
    pub(super) p_memsz: u64,
    pub(super) bytes: Vec<u8>,
}

impl SegSpec {
    pub(super) fn pt_load(p_offset: u64, p_vaddr: u64, bytes: Vec<u8>) -> Self {
        let len = bytes.len() as u64;
        Self {
            p_type: PT_LOAD,
            p_offset,
            p_vaddr,
            p_filesz: len,
            p_memsz: len,
            bytes,
        }
    }
}

pub(super) fn put_be_u16(buf: &mut [u8], off: usize, v: u16) {
    buf[off..off + 2].copy_from_slice(&v.to_be_bytes());
}
pub(super) fn put_be_u32(buf: &mut [u8], off: usize, v: u32) {
    buf[off..off + 4].copy_from_slice(&v.to_be_bytes());
}
pub(super) fn put_be_u64(buf: &mut [u8], off: usize, v: u64) {
    buf[off..off + 8].copy_from_slice(&v.to_be_bytes());
}

/// Build an ELF64-MSB blob containing the given segments. Phdrs
/// land immediately after the 64-byte ELF header; segment bytes
/// land at each spec's `p_offset`.
pub(super) fn build_elf64_be(segs: &[SegSpec]) -> Vec<u8> {
    let phentsize = ELF_PHENTSIZE as u16;
    let phoff: u64 = ELF_HEADER_SIZE as u64;
    let phnum: u16 = segs.len() as u16;
    let phdr_table_end = phoff + (phentsize as u64) * (phnum as u64);
    let mut file_end: u64 = phdr_table_end;
    for seg in segs {
        let segend = seg.p_offset + seg.bytes.len() as u64;
        if segend > file_end {
            file_end = segend;
        }
    }
    let mut data = vec![0u8; file_end as usize];
    data[0..4].copy_from_slice(b"\x7fELF");
    data[4] = ELFCLASS64;
    data[5] = ELFDATA2MSB;
    data[6] = EV_CURRENT;
    put_be_u16(&mut data, 16, ET_EXEC);
    put_be_u16(&mut data, 18, EM_PPC64);
    put_be_u32(&mut data, 20, 1); // e_version (u32 form, must mirror EV_CURRENT)
    put_be_u64(&mut data, 32, phoff);
    put_be_u16(&mut data, 52, ELF_HEADER_SIZE as u16);
    put_be_u16(&mut data, 54, phentsize);
    put_be_u16(&mut data, 56, phnum);

    for (i, seg) in segs.iter().enumerate() {
        let base = phoff as usize + i * phentsize as usize;
        put_be_u32(&mut data, base, seg.p_type);
        put_be_u32(&mut data, base + 4, PF_R | PF_X);
        put_be_u64(&mut data, base + 8, seg.p_offset);
        put_be_u64(&mut data, base + 16, seg.p_vaddr);
        put_be_u64(&mut data, base + 24, seg.p_vaddr); // p_paddr
        put_be_u64(&mut data, base + 32, seg.p_filesz);
        put_be_u64(&mut data, base + 40, seg.p_memsz);
        put_be_u64(&mut data, base + 48, 0); // p_align
        let off = seg.p_offset as usize;
        data[off..off + seg.bytes.len()].copy_from_slice(&seg.bytes);
    }
    data
}

/// Re-exports of the PPC ISA byte-encoded instructions shared across
/// disasm tests.
pub(super) use cellgov_ps3_abi::ppc_isa::{PPC_BLR_BYTES, PPC_NOP_BYTES};
