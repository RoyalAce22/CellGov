//! ELF64 big-endian container writer for PPU microtest binaries.
//!
//! Emits two PT_LOAD segments (code R+X, data R+W) and an optional
//! PROC_PARAM segment (p_type 0x60000001) that lv2 scans for
//! `process_param_t` during ELF load.

use cellgov_ps3_abi::format::elf::{
    ELFCLASS64, ELFDATA2MSB, ELF_HEADER_SIZE, ELF_MAGIC, ELF_PHENTSIZE, EM_PPC64, ET_EXEC,
    EV_CURRENT, PF_R, PF_W, PF_X, PT_LOAD, PT_PROC_PARAM, SYS_PROCESS_PARAM_MAGIC,
    SYS_PROCESS_PARAM_VERSION_330_0,
};

/// `e_ehsize` field value.
const ELF64_EHDR_SIZE: u16 = ELF_HEADER_SIZE as u16;

/// `e_phentsize` field value.
const ELF64_PHDR_SIZE: u16 = ELF_PHENTSIZE as u16;

pub use cellgov_ps3_abi::format::elf::PROC_PARAM_SIZE;

/// Build a `process_param_t` structure (32 bytes, big-endian).
///
/// The leading `size` is the record's own extent, and downstream
/// consumers take it literally -- the compare classifier derives
/// `sys_proc_param_range` from it, so a `size` larger than the bytes
/// emitted would extend that range over memory this record does not
/// own and classify a neighbour's divergence as this record's.
pub fn proc_param(sdk_version: u32) -> Vec<u8> {
    let mut buf = Vec::with_capacity(PROC_PARAM_SIZE as usize);
    write_u32(&mut buf, PROC_PARAM_SIZE as u32);
    write_u32(&mut buf, SYS_PROCESS_PARAM_MAGIC);
    write_u32(&mut buf, SYS_PROCESS_PARAM_VERSION_330_0);
    write_u32(&mut buf, sdk_version);
    write_u32(&mut buf, 1001);
    write_u32(&mut buf, 0x00100000);
    write_u32(&mut buf, 0x00100000);
    write_u32(&mut buf, 0);
    debug_assert_eq!(buf.len() as u64, PROC_PARAM_SIZE);
    buf
}

/// Build a PPU ELF64 with code, data, and optional PROC_PARAM.
///
/// `entry_vaddr` points at the OPD in the code segment.
///
/// # Panics
///
/// When `proc_param_offset` is `Some`, a `process_param_t` of length
/// [`PROC_PARAM_SIZE`] must fit inside `data` at that offset. Also
/// panics if any emitted segment's `[p_vaddr, p_vaddr + p_memsz)`
/// would run past the end of the u64 address space.
pub fn build(
    entry_vaddr: u64,
    code_vaddr: u64,
    code: &[u8],
    data_vaddr: u64,
    data: &[u8],
    proc_param_offset: Option<u64>,
) -> Vec<u8> {
    // A PT_LOAD whose [p_vaddr, p_vaddr + p_memsz) wraps the address
    // space is not loadable: the loader's own range arithmetic would
    // overflow before it ever saw the segment. Refuse here rather than
    // emit an ELF whose segment table only looks well-formed field by
    // field.
    assert_segment_fits("code", code_vaddr, code.len());
    assert_segment_fits("data", data_vaddr, data.len());

    // The PT_PROC_PARAM segment declares p_filesz = PROC_PARAM_SIZE at
    // `data_file_offset + pp_offset`. A record that does not fit inside
    // `data` would put [p_offset, p_offset + p_filesz) past end of file
    // -- the same malformed-segment shape the disasm PT_LOAD parser
    // rejects as SegmentTruncated.
    if let Some(pp_offset) = proc_param_offset {
        assert!(
            pp_offset
                .checked_add(PROC_PARAM_SIZE)
                .is_some_and(|end| end <= data.len() as u64),
            "proc_param_offset 0x{pp_offset:x} + {PROC_PARAM_SIZE} bytes runs past the \
             {}-byte data segment",
            data.len()
        );
    }

    let phnum: u16 = if proc_param_offset.is_some() { 3 } else { 2 };
    let phoff: u64 = ELF64_EHDR_SIZE as u64;

    let code_file_offset = align_up(phoff + (phnum as u64) * (ELF64_PHDR_SIZE as u64), 16);
    let data_file_offset = align_up(code_file_offset + code.len() as u64, 16);
    let total_size = data_file_offset + data.len() as u64;

    let mut buf = Vec::with_capacity(total_size as usize);

    // ELF64 header
    buf.extend_from_slice(&ELF_MAGIC);
    buf.push(ELFCLASS64);
    buf.push(ELFDATA2MSB);
    buf.push(EV_CURRENT);
    buf.push(0x66); // EI_OSABI: lv2
    buf.extend_from_slice(&[0u8; 8]);
    write_u16(&mut buf, ET_EXEC);
    write_u16(&mut buf, EM_PPC64);
    write_u32(&mut buf, EV_CURRENT.into());
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
    assert_eq!(buf.len(), ELF_HEADER_SIZE);

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
        let pp_vaddr = data_vaddr
            .checked_add(pp_offset)
            .expect("proc_param vaddr overflows u64");
        let pp_file_offset = data_file_offset
            .checked_add(pp_offset)
            .expect("proc_param file offset overflows u64");
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

/// # Panics
///
/// If `[vaddr, vaddr + len)` leaves the u64 address space.
fn assert_segment_fits(which: &str, vaddr: u64, len: usize) {
    assert!(
        vaddr.checked_add(len as u64).is_some(),
        "{which} segment vaddr overflows u64: 0x{vaddr:x} + {len} bytes"
    );
}

/// # Panics
///
/// If rounding `value` up to `align` leaves the u64 range.
fn align_up(value: u64, align: u64) -> u64 {
    value
        .checked_add(align - 1)
        .expect("align_up overflows u64")
        & !(align - 1)
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
#[path = "tests/elf64_tests.rs"]
mod tests;
