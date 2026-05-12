//! Synthetic ELF builders shared by `elf::tests` and `stream::tests`.

#![cfg(test)]

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
            p_type: 1,
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
    const PHENTSIZE: u16 = 56;
    let phoff: u64 = 64;
    let phnum: u16 = segs.len() as u16;
    let phdr_table_end = phoff + (PHENTSIZE as u64) * (phnum as u64);
    let mut file_end: u64 = phdr_table_end;
    for seg in segs {
        let segend = seg.p_offset + seg.bytes.len() as u64;
        if segend > file_end {
            file_end = segend;
        }
    }
    let mut data = vec![0u8; file_end as usize];
    data[0..4].copy_from_slice(b"\x7fELF");
    data[4] = 2; // EI_CLASS = ELFCLASS64
    data[5] = 2; // EI_DATA  = ELFDATA2MSB
    data[6] = 1; // EI_VERSION
    put_be_u16(&mut data, 16, 2); // e_type = ET_EXEC
    put_be_u16(&mut data, 18, 21); // e_machine = EM_PPC64
    put_be_u32(&mut data, 20, 1); // e_version
    put_be_u64(&mut data, 32, phoff);
    put_be_u16(&mut data, 52, 64); // e_ehsize
    put_be_u16(&mut data, 54, PHENTSIZE);
    put_be_u16(&mut data, 56, phnum);

    for (i, seg) in segs.iter().enumerate() {
        let base = phoff as usize + i * PHENTSIZE as usize;
        put_be_u32(&mut data, base, seg.p_type);
        put_be_u32(&mut data, base + 4, 5); // p_flags = PF_R | PF_X
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

/// `nop` (ori 0,0,0) = 0x60000000.
pub(super) const NOP: [u8; 4] = [0x60, 0x00, 0x00, 0x00];
/// `blr` = 0x4E800020.
pub(super) const BLR: [u8; 4] = [0x4E, 0x80, 0x00, 0x20];
