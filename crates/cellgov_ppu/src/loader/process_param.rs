//! `sys_process_param_t` discovery.

use cellgov_mem::be::read_u32;
use cellgov_ps3_abi::format::elf::SYS_PROCESS_PARAM_MAGIC;

use super::phdr::read_pt_loads;

/// Parsed `sys_process_param_t`. The caller passes `malloc_pagesize`
/// into the game entry via `r12` so the CRT0 sizes its allocator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SysProcessParam {
    /// SDK version that built the ELF (e.g., 0x150004 = SDK 1.5.0.4).
    pub sdk_version: u32,
    /// Primary PPU thread priority (0..3071).
    pub primary_prio: i32,
    /// Primary PPU thread stack size in bytes.
    pub primary_stacksize: u32,
    /// libc malloc page size (0x10000 = 64KB, 0x100000 = 1MB, 0 = unset).
    pub malloc_pagesize: u32,
    /// PPC segment mode (0 = default, 1 = OVLM).
    pub ppc_seg: u32,
    /// Guest address where the struct lives after the ELF is loaded,
    /// derived by mapping the struct's file offset through the
    /// containing PT_LOAD segment.
    pub guest_addr: u64,
    /// Value of the on-disk `size` field at struct offset 0
    /// (typically 0x30 or 0x40 across observed SDKs).
    pub struct_size: u32,
}

/// Locate the PT_LOAD whose file range covers `file_off` and return
/// the corresponding guest virtual address, or `None` if no PT_LOAD
/// covers the offset.
pub(super) fn pt_load_file_to_guest(data: &[u8], file_off: usize) -> Option<u64> {
    let file_off = file_off as u64;
    read_pt_loads(data).ok()?.into_iter().find_map(|seg| {
        let delta = file_off.checked_sub(seg.file_offset)?;
        (delta < seg.filesz).then(|| seg.vaddr.checked_add(delta))?
    })
}

/// Locate `.sys_proc_param` by scanning for its magic (avoids parsing
/// section headers). `None` if the magic is absent. Matches outside any
/// PT_LOAD file range are rejected so a stray byte sequence in a string
/// table, debug section, or embedded asset cannot masquerade as a real
/// `sys_process_param_t`.
pub fn find_sys_process_param(data: &[u8]) -> Option<SysProcessParam> {
    let magic_bytes = SYS_PROCESS_PARAM_MAGIC.to_be_bytes();
    // Struct: { u32 size, magic, version, sdk_version, i32 primary_prio,
    //           u32 primary_stacksize, malloc_pagesize, ppc_seg }. Magic
    // is at offset 4 of the struct.
    let mut idx = 0;
    while idx + 4 <= data.len() {
        let rel = data[idx..].windows(4).position(|w| w == magic_bytes)?;
        let s = idx + rel;
        if s < 4 || s + 28 > data.len() {
            idx = s + 4;
            continue;
        }
        let start = s - 4;
        let size = read_u32(data, start);
        if size < 0x20 {
            idx = s + 4;
            continue;
        }
        let Some(guest_addr) = pt_load_file_to_guest(data, start) else {
            idx = s + 4;
            continue;
        };
        return Some(SysProcessParam {
            sdk_version: read_u32(data, start + 12),
            primary_prio: read_u32(data, start + 16) as i32,
            primary_stacksize: read_u32(data, start + 20),
            malloc_pagesize: read_u32(data, start + 24),
            ppc_seg: read_u32(data, start + 28),
            guest_addr,
            struct_size: size,
        });
    }
    None
}
