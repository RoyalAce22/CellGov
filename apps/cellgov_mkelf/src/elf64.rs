//! ELF64 big-endian container writer for PPU microtest binaries.
//!
//! Emits two PT_LOAD segments (code R+X, data R+W) and an optional
//! PROC_PARAM segment (p_type 0x60000001) that lv2 scans for
//! `process_param_t` during ELF load.

const ELFCLASS64: u8 = 2;
const ELFDATA2MSB: u8 = 2;
const EV_CURRENT: u8 = 1;
const ET_EXEC: u16 = 2;
const EM_PPC64: u16 = 21;
const ELF64_EHDR_SIZE: u16 = 64;
const ELF64_PHDR_SIZE: u16 = 56;

const PT_LOAD: u32 = 1;
const PT_PROC_PARAM: u32 = 0x60000001;

const PF_X: u32 = 1;
const PF_W: u32 = 2;
const PF_R: u32 = 4;

/// Size of a `process_param_t` record in the data segment.
pub const PROC_PARAM_SIZE: u64 = 32;

/// Build a `process_param_t` structure (32 bytes, big-endian).
pub fn proc_param(sdk_version: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(PROC_PARAM_SIZE as usize);
    write_u32(&mut buf, 0x40);
    write_u32(&mut buf, 0x13bcc5f6);
    write_u32(&mut buf, sdk_version);
    write_u32(&mut buf, sdk_version);
    write_u32(&mut buf, 1001);
    write_u32(&mut buf, 0x00100000);
    write_u32(&mut buf, 0x00100000);
    write_u32(&mut buf, 0);
    buf
}

/// Build a PPU ELF64 with code, data, and optional PROC_PARAM.
///
/// `entry_vaddr` points at the OPD in the code segment. When
/// `proc_param_offset` is `Some`, the data at `data_vaddr + offset`
/// must be a valid `process_param_t` of length [`PROC_PARAM_SIZE`].
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

    let code_file_offset = align_up(phoff + (phnum as u64) * (ELF64_PHDR_SIZE as u64), 16);
    let data_file_offset = align_up(code_file_offset + code.len() as u64, 16);
    let total_size = data_file_offset + data.len() as u64;

    let mut buf = Vec::with_capacity(total_size as usize);

    // ELF64 header
    buf.extend_from_slice(&[0x7f, b'E', b'L', b'F']);
    buf.push(ELFCLASS64);
    buf.push(ELFDATA2MSB);
    buf.push(EV_CURRENT);
    buf.push(0x66); // EI_OSABI: lv2
    buf.extend_from_slice(&[0u8; 8]);
    write_u16(&mut buf, ET_EXEC);
    write_u16(&mut buf, EM_PPC64);
    write_u32(&mut buf, 1);
    write_u64(&mut buf, entry_vaddr);
    write_u64(&mut buf, phoff);
    write_u64(&mut buf, 0);
    write_u32(&mut buf, 0);
    write_u16(&mut buf, ELF64_EHDR_SIZE);
    write_u16(&mut buf, ELF64_PHDR_SIZE);
    write_u16(&mut buf, phnum);
    write_u16(&mut buf, 0);
    write_u16(&mut buf, 0);
    write_u16(&mut buf, 0);
    assert_eq!(buf.len(), 64);

    // PT_LOAD code (R+X)
    write_u32(&mut buf, PT_LOAD);
    write_u32(&mut buf, PF_R | PF_X);
    write_u64(&mut buf, code_file_offset);
    write_u64(&mut buf, code_vaddr);
    write_u64(&mut buf, code_vaddr);
    write_u64(&mut buf, code.len() as u64);
    write_u64(&mut buf, code.len() as u64);
    write_u64(&mut buf, 16);

    // PT_LOAD data (R+W)
    write_u32(&mut buf, PT_LOAD);
    write_u32(&mut buf, PF_R | PF_W);
    write_u64(&mut buf, data_file_offset);
    write_u64(&mut buf, data_vaddr);
    write_u64(&mut buf, data_vaddr);
    write_u64(&mut buf, data.len() as u64);
    write_u64(&mut buf, data.len() as u64);
    write_u64(&mut buf, 16);

    if let Some(pp_offset) = proc_param_offset {
        let pp_vaddr = data_vaddr + pp_offset;
        let pp_file_offset = data_file_offset + pp_offset;
        write_u32(&mut buf, PT_PROC_PARAM);
        write_u32(&mut buf, PF_R);
        write_u64(&mut buf, pp_file_offset);
        write_u64(&mut buf, pp_vaddr);
        write_u64(&mut buf, pp_vaddr);
        write_u64(&mut buf, PROC_PARAM_SIZE);
        write_u64(&mut buf, PROC_PARAM_SIZE);
        write_u64(&mut buf, 4);
    }

    buf.resize(code_file_offset as usize, 0);
    buf.extend_from_slice(code);

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
