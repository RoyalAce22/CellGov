//! Minimal ELF64 big-endian builder for PS3 PPU executables.
//!
//! Produces a valid ELF64 that RPCS3 can load directly (no SELF
//! wrapper needed). Supports two PT_LOAD segments (code R+X, data
//! R+W) and an optional PROC_PARAM segment (type 0x60000001) for
//! process parameters.

/// ELF64 header constants.
const ELFCLASS64: u8 = 2;
const ELFDATA2MSB: u8 = 2; // big-endian
const EV_CURRENT: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_PPC64: u16 = 21;
const ELF64_EHDR_SIZE: u16 = 64;
const ELF64_PHDR_SIZE: u16 = 56;

/// Program header types.
const PT_LOAD: u32 = 1;
const PT_PROC_PARAM: u32 = 0x60000001;

/// Program header flags.
const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

/// PS3 process_param_t size (8 fields x 4 bytes = 32 bytes).
pub const PROC_PARAM_SIZE: u64 = 32;

/// Build a `process_param_t` structure (32 bytes, big-endian).
pub fn proc_param(sdk_version: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(PROC_PARAM_SIZE as usize);
    write_u32(&mut buf, 0x40); // size (padded)
    write_u32(&mut buf, 0x13bcc5f6); // magic
    write_u32(&mut buf, sdk_version); // version
    write_u32(&mut buf, sdk_version); // sdk_version
    write_u32(&mut buf, 1001); // primary_prio
    write_u32(&mut buf, 0x00100000); // primary_stacksize (1MB)
    write_u32(&mut buf, 0x00100000); // malloc_pagesize (1MB)
    write_u32(&mut buf, 0); // ppc_seg
    buf
}

/// Build a PS3 PPU ELF64 with code, data, and optional PROC_PARAM.
///
/// - `entry_vaddr`: virtual address of the entry point (OPD).
/// - `code_vaddr`: virtual address where code segment loads.
/// - `code`: raw code bytes (OPD at the start).
/// - `data_vaddr`: virtual address where data segment loads.
/// - `data`: raw data bytes (may include process_param_t).
/// - `proc_param_offset`: if `Some(offset)`, emits a PROC_PARAM
///   program header pointing to `data_vaddr + offset` with size
///   `PROC_PARAM_SIZE`. The data at that offset must be a valid
///   `process_param_t`.
pub fn build(
    entry_vaddr: u64,
    code_vaddr: u64,
    code: &[u8],
    data_vaddr: u64,
    data: &[u8],
    proc_param_offset: Option<u64>,
) -> Vec<u8> {
    let phnum: u16 = if proc_param_offset.is_some() { 3 } else { 2 };
    let phoff: u64 = ELF64_EHDR_SIZE as u64;

    // Code segment starts after headers.
    let code_file_offset = align_up(phoff + (phnum as u64) * (ELF64_PHDR_SIZE as u64), 16);
    // Data segment starts after code.
    let data_file_offset = align_up(code_file_offset + code.len() as u64, 16);
    let total_size = data_file_offset + data.len() as u64;

    let mut buf = Vec::with_capacity(total_size as usize);

    // ELF64 header (64 bytes)
    buf.extend_from_slice(&[0x7f, b'E', b'L', b'F']); // e_ident magic
    buf.push(ELFCLASS64); // EI_CLASS
    buf.push(ELFDATA2MSB); // EI_DATA
    buf.push(EV_CURRENT); // EI_VERSION
    buf.push(0x66); // EI_OSABI: lv2
    buf.extend_from_slice(&[0u8; 8]); // EI_ABIVERSION + padding
    write_u16(&mut buf, ET_EXEC); // e_type
    write_u16(&mut buf, EM_PPC64); // e_machine
    write_u32(&mut buf, 1); // e_version
    write_u64(&mut buf, entry_vaddr); // e_entry (OPD address)
    write_u64(&mut buf, phoff); // e_phoff
    write_u64(&mut buf, 0); // e_shoff (no sections)
    write_u32(&mut buf, 0); // e_flags
    write_u16(&mut buf, ELF64_EHDR_SIZE); // e_ehsize
    write_u16(&mut buf, ELF64_PHDR_SIZE); // e_phentsize
    write_u16(&mut buf, phnum); // e_phnum
    write_u16(&mut buf, 0); // e_shentsize
    write_u16(&mut buf, 0); // e_shnum
    write_u16(&mut buf, 0); // e_shstrndx
    assert_eq!(buf.len(), 64);

    // Program header 0: code segment (R+X)
    write_u32(&mut buf, PT_LOAD); // p_type
    write_u32(&mut buf, PF_R | PF_X); // p_flags
    write_u64(&mut buf, code_file_offset); // p_offset
    write_u64(&mut buf, code_vaddr); // p_vaddr
    write_u64(&mut buf, code_vaddr); // p_paddr
    write_u64(&mut buf, code.len() as u64); // p_filesz
    write_u64(&mut buf, code.len() as u64); // p_memsz
    write_u64(&mut buf, 16); // p_align

    // Program header 1: data segment (R+W)
    write_u32(&mut buf, PT_LOAD); // p_type
    write_u32(&mut buf, PF_R | PF_W); // p_flags
    write_u64(&mut buf, data_file_offset); // p_offset
    write_u64(&mut buf, data_vaddr); // p_vaddr
    write_u64(&mut buf, data_vaddr); // p_paddr
    write_u64(&mut buf, data.len() as u64); // p_filesz
    write_u64(&mut buf, data.len() as u64); // p_memsz
    write_u64(&mut buf, 16); // p_align

    // Program header 2 (optional): PROC_PARAM
    if let Some(pp_offset) = proc_param_offset {
        let pp_vaddr = data_vaddr + pp_offset;
        let pp_file_offset = data_file_offset + pp_offset;
        write_u32(&mut buf, PT_PROC_PARAM); // p_type
        write_u32(&mut buf, PF_R); // p_flags
        write_u64(&mut buf, pp_file_offset); // p_offset
        write_u64(&mut buf, pp_vaddr); // p_vaddr
        write_u64(&mut buf, pp_vaddr); // p_paddr
        write_u64(&mut buf, PROC_PARAM_SIZE); // p_filesz
        write_u64(&mut buf, PROC_PARAM_SIZE); // p_memsz
        write_u64(&mut buf, 4); // p_align
    }

    // Pad to code offset
    buf.resize(code_file_offset as usize, 0);
    buf.extend_from_slice(code);

    // Pad to data offset
    buf.resize(data_file_offset as usize, 0);
    buf.extend_from_slice(data);

    buf
}

fn align_up(value: u64, align: u64) -> u64 {
    (value + align - 1) & !(align - 1)
}

fn write_u16(buf: &mut Vec<u8>, v: u16) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn write_u32(buf: &mut Vec<u8>, v: u32) {
    buf.extend_from_slice(&v.to_be_bytes());
}

fn write_u64(buf: &mut Vec<u8>, v: u64) {
    buf.extend_from_slice(&v.to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_produces_valid_elf_magic() {
        let elf = build(0x10000, 0x10000, &[0; 16], 0x20000, &[0; 16], None);
        assert_eq!(&elf[0..4], b"\x7fELF");
    }

    #[test]
    fn build_produces_big_endian_ppc64() {
        let elf = build(0x10000, 0x10000, &[0; 16], 0x20000, &[0; 16], None);
        assert_eq!(elf[4], ELFCLASS64);
        assert_eq!(elf[5], ELFDATA2MSB);
        assert_eq!(&elf[18..20], &[0x00, 0x15]);
    }

    #[test]
    fn build_entry_point_matches() {
        let elf = build(0x10000, 0x10000, &[0; 16], 0x20000, &[0; 16], None);
        let entry = u64::from_be_bytes(elf[24..32].try_into().unwrap());
        assert_eq!(entry, 0x10000);
    }

    #[test]
    fn build_without_proc_param_has_two_phdrs() {
        let elf = build(0x10000, 0x10000, &[0; 16], 0x20000, &[0; 16], None);
        let phnum = u16::from_be_bytes(elf[56..58].try_into().unwrap());
        assert_eq!(phnum, 2);
    }

    #[test]
    fn build_with_proc_param_has_three_phdrs() {
        let pp = proc_param(0x00360001);
        let mut data = vec![0u8; 8];
        let pp_offset = data.len() as u64;
        data.extend_from_slice(&pp);
        let elf = build(0x10000, 0x10000, &[0; 16], 0x20000, &data, Some(pp_offset));
        let phnum = u16::from_be_bytes(elf[56..58].try_into().unwrap());
        assert_eq!(phnum, 3);
    }

    #[test]
    fn proc_param_has_correct_magic() {
        let pp = proc_param(0x00360001);
        let magic = u32::from_be_bytes(pp[4..8].try_into().unwrap());
        assert_eq!(magic, 0x13bcc5f6);
    }

    #[test]
    fn code_bytes_appear_in_output() {
        let code = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let elf = build(0x10000, 0x10000, &code, 0x20000, &[0; 4], None);
        assert!(elf.windows(4).any(|w| w == [0xDE, 0xAD, 0xBE, 0xEF]));
    }

    #[test]
    fn data_bytes_appear_in_output() {
        let data = vec![0xCA, 0xFE, 0xBA, 0xBE];
        let elf = build(0x10000, 0x10000, &[0; 4], 0x20000, &data, None);
        assert!(elf.windows(4).any(|w| w == [0xCA, 0xFE, 0xBA, 0xBE]));
    }
}
