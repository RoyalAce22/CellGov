//! PT_TLS discovery.

use cellgov_mem::be::{read_u16, read_u32, read_u64};
use cellgov_ps3_abi::format::elf::{ELF_HEADER_SIZE, ELF_MAGIC, PT_TLS};

use super::phdr::ph_slot_base;

/// TLS segment info extracted from an ELF's PT_TLS program header.
#[derive(Debug, Clone, Copy)]
pub struct TlsInfo {
    /// Virtual address of the TLS template in guest memory.
    pub vaddr: u64,
    /// Size of initialized TLS data (from ELF file).
    pub filesz: u64,
    /// Total TLS memory size per thread (including BSS).
    pub memsz: u64,
}

/// Full PT_TLS layout for per-thread TLS block reconstruction.
#[derive(Debug, Clone, Copy)]
pub struct TlsProgramHeader {
    /// Offset into the ELF file where the initialized bytes start.
    pub file_offset: u64,
    /// Virtual address of the primary thread's TLS block.
    pub vaddr: u64,
    /// Count of initialized bytes (the `.tdata` payload length).
    pub filesz: u64,
    /// Total per-thread size (filesz plus `.tbss` zero-init tail).
    pub memsz: u64,
    /// Required alignment for per-thread TLS blocks.
    pub align: u64,
}

/// PT_TLS segment info, or `None` if the ELF has no TLS segment.
pub fn find_tls_segment(data: &[u8]) -> Option<TlsInfo> {
    if data.len() < ELF_HEADER_SIZE || data[0..4] != ELF_MAGIC || data[4] != 2 || data[5] != 2 {
        return None;
    }
    let phoff = read_u64(data, 32) as usize;
    let phentsize = read_u16(data, 54) as usize;
    let phnum = read_u16(data, 56) as usize;

    for i in 0..phnum {
        let base = ph_slot_base(data.len(), phoff, phentsize, i).ok()?;
        if read_u32(data, base) == PT_TLS {
            return Some(TlsInfo {
                vaddr: read_u64(data, base + 16),
                filesz: read_u64(data, base + 32),
                memsz: read_u64(data, base + 40),
            });
        }
    }
    None
}

/// PT_TLS program header (including `p_offset` and `p_align`), or
/// `None` if absent.
pub fn find_tls_program_header(data: &[u8]) -> Option<TlsProgramHeader> {
    if data.len() < ELF_HEADER_SIZE || data[0..4] != ELF_MAGIC || data[4] != 2 || data[5] != 2 {
        return None;
    }
    let phoff = read_u64(data, 32) as usize;
    let phentsize = read_u16(data, 54) as usize;
    let phnum = read_u16(data, 56) as usize;

    for i in 0..phnum {
        let base = ph_slot_base(data.len(), phoff, phentsize, i).ok()?;
        if read_u32(data, base) == PT_TLS {
            return Some(TlsProgramHeader {
                file_offset: read_u64(data, base + 8),
                vaddr: read_u64(data, base + 16),
                filesz: read_u64(data, base + 32),
                memsz: read_u64(data, base + 40),
                align: read_u64(data, base + 48),
            });
        }
    }
    None
}
