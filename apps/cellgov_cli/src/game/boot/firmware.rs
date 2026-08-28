//! The firmware PRX set the title's import tables name, and the
//! heap floor its extent forces.

use std::time::{Duration, Instant};

use super::types::{u32_or_die, PrepareOptions};
use crate::cli::exit::die;
use crate::game::prx::{
    install_unresolved_trampolines_only, load_firmware_set_bound, HostLinkMaps, PrxLoadInfo,
    VerifiedFirmware,
};

/// The firmware modules resident in guest memory, with the identity
/// and link maps the LV2 host binds from them.
pub(super) struct FirmwareSet {
    pub prx_modules: Vec<PrxLoadInfo>,
    /// `None` when no `firmware.toml` covered the loaded set.
    pub verified: Option<VerifiedFirmware>,
    pub host_link: HostLinkMaps,
    pub t_hle_bind: Duration,
    pub t_prx_load: Duration,
}

/// Parse the title's import tables and load the firmware modules they
/// name into `mem`, above `code_floor`.
///
/// A boot that loads no firmware still gets trampolines over its
/// unresolved imports, so a call through one produces a structured
/// fault.
pub(super) fn load_firmware_set(
    opts: &PrepareOptions<'_>,
    elf_data: &[u8],
    mem: &mut cellgov_mem::GuestMemory,
    code_floor: u32,
    t_start: Instant,
) -> FirmwareSet {
    let modules = cellgov_ppu::prx::parse_imports(elf_data)
        .unwrap_or_else(|e| die(&format!("imports: parse failed: {e:?}")));
    if opts.print_banner {
        println!("imports: {} modules", modules.len());
        for m in &modules {
            let first_stub = m.functions.first().map(|f| f.stub_addr).unwrap_or(0);
            println!(
                "  {}: {} functions, first stub at 0x{:x}",
                m.name,
                m.functions.len(),
                first_stub
            );
        }
    }
    let t_hle_bind = t_start.elapsed();

    let (mut prx_modules, verified, mut host_link) = load_firmware_set_bound(
        opts.firmware_dir,
        &modules,
        mem,
        code_floor,
        matches!(
            opts.title.source,
            crate::game::manifest::GameSource::FirmwareExec { .. }
        ),
    );
    let t_prx_load = t_start.elapsed();
    if prx_modules.is_empty() {
        let (info, requesters) =
            install_unresolved_trampolines_only(&modules, mem, code_floor as u64);
        if let Some(info) = info {
            prx_modules.push(info);
        }
        host_link.unresolved_requesters = requesters;
    }

    FirmwareSet {
        prx_modules,
        verified,
        host_link,
        t_hle_bind,
        t_prx_load,
    }
}

/// Where the guest heap starts, and the three floors it had to clear.
pub(super) struct MemoryPlacement {
    /// Highest of the three floors, before page rounding. Nothing
    /// below it is heap, and nothing at or above it is code.
    pub alloc_floor: usize,
    /// `sys_memory_allocate`'s first address.
    pub alloc_base: u32,
    pub user_region_end: usize,
    pub prx_region_end: usize,
}

/// Place the guest heap above everything already resident.
///
/// The base must clear the title's PT_LOAD ranges, the HLE trampoline
/// and OPD-body span, and the firmware set's load region -- an
/// allocation inside any of the three would hand the title an address
/// that already holds code.
pub(super) fn place_guest_heap(
    elf_data: &[u8],
    prx_modules: &[PrxLoadInfo],
    code_floor: u32,
) -> MemoryPlacement {
    let user_region_end = crate::game::observation::elf_user_region_end(elf_data);
    let prx_region_end: usize = prx_modules
        .iter()
        .map(|p| p.data_end as usize)
        .max()
        .unwrap_or(0);
    let alloc_floor = user_region_end.max(code_floor as usize).max(prx_region_end);
    let alloc_base = {
        let rounded = alloc_floor.checked_add(0xFFFF).unwrap_or_else(|| {
            die(&format!(
                "alloc_floor=0x{alloc_floor:x} + 0xFFFF overflows usize"
            ))
        }) & !0xFFFF;
        u32_or_die("alloc_base", rounded.max(0x0001_0000) as u64)
    };
    MemoryPlacement {
        alloc_floor,
        alloc_base,
        user_region_end,
        prx_region_end,
    }
}

#[cfg(test)]
#[path = "tests/firmware_tests.rs"]
mod tests;
