//! The firmware PRX set the title's import tables name, and the TLS
//! area and heap floor its extent forces.

use std::time::{Duration, Instant};

use super::types::{BootServices, DiagnosticOptions, TitleOptions};
use crate::error::narrow_u32;
use crate::prx::{
    install_unresolved_trampolines_only, load_firmware_set_bound, FirmwareLoadError, HostLinkMaps,
    PrxLoadInfo, TlsArea, VerifiedFirmware,
};
use crate::BootError;

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
///
/// # Errors
///
/// The title's import tables do not parse, or the firmware set does
/// not load; see [`FirmwareLoadError`].
pub(super) fn load_firmware_set(
    title: &TitleOptions<'_>,
    diagnostics: &DiagnosticOptions<'_>,
    services: &BootServices,
    elf_data: &[u8],
    mem: &mut cellgov_mem::GuestMemory,
    code_floor: u32,
    t_start: Instant,
) -> Result<FirmwareSet, BootError> {
    let sink = services.sink();
    let modules = cellgov_ppu::prx::parse_imports(elf_data)
        .map_err(|source| FirmwareLoadError::ImportParse { source })?;
    if diagnostics.print_banner {
        sink.note(&format!("imports: {} modules", modules.len()));
        for m in &modules {
            let first_stub = m.functions.first().map(|f| f.stub_addr).unwrap_or(0);
            sink.note(&format!(
                "  {}: {} functions, first stub at 0x{:x}",
                m.name,
                m.functions.len(),
                first_stub
            ));
        }
    }
    let t_hle_bind = t_start.elapsed();

    let (mut prx_modules, verified, mut host_link) = load_firmware_set_bound(
        title.firmware_dir,
        &modules,
        mem,
        code_floor,
        title.identity.overrides.prx_base,
        title.manifest.ships_in_firmware(),
        sink,
        services.keys.as_ref(),
    )?;
    let t_prx_load = t_start.elapsed();
    services.taps.firmware_bound(0, &host_link.exports, mem);
    if prx_modules.is_empty() {
        let (info, requesters) =
            install_unresolved_trampolines_only(&modules, mem, code_floor as u64, sink)?;
        if let Some(info) = info {
            prx_modules.push(info);
        }
        host_link.unresolved_requesters = requesters;
    }

    Ok(FirmwareSet {
        prx_modules,
        verified,
        host_link,
        t_hle_bind,
        t_prx_load,
    })
}

/// Where the TLS area and the guest heap start, and the three floors
/// they had to clear.
pub(super) struct MemoryPlacement {
    /// Highest of the three floors, before page rounding. Nothing
    /// below it is heap, and nothing at or above it is code.
    pub alloc_floor: usize,
    /// The main thread's TLS area, on the first 64 KB page at or above
    /// `alloc_floor`.
    pub tls: TlsArea,
    /// `sys_memory_allocate`'s first address, the end of `tls`.
    pub alloc_base: u32,
    pub user_region_end: usize,
    pub prx_region_end: usize,
}

/// Place the TLS area and then the guest heap above everything already
/// resident.
///
/// Both clear three floors, because TLS or an allocation inside any of
/// them would overwrite code:
///
/// - the title's PT_LOAD ranges,
/// - the HLE trampoline and OPD-body span,
/// - the firmware set's load region.
///
/// The TLS area takes the first 64 KB page at or above the highest
/// floor, and the heap starts where the area ends. This order matches
/// the console: there, the first 64 KB allocation follows the
/// user-memory block that holds the main thread's TLS. The addresses
/// differ from the console's; see [`TlsArea`].
///
/// # Errors
///
/// - The TLS area does not fit above the floor.
/// - The heap base does not fit a `u32`.
pub(super) fn place_guest_heap(
    elf_data: &[u8],
    prx_modules: &[PrxLoadInfo],
    code_floor: u32,
    sink: &dyn crate::BootSink,
) -> Result<MemoryPlacement, BootError> {
    let user_region_end = crate::observation::elf_user_region_end(elf_data, sink);
    let prx_region_end: usize = prx_modules
        .iter()
        .map(|p| p.data_end as usize)
        .max()
        .unwrap_or(0);
    let alloc_floor = user_region_end.max(code_floor as usize).max(prx_region_end);
    // The floor of one page keeps the area off address 0, which the
    // boot path reads as null.
    let tls = TlsArea::above((alloc_floor as u64).max(TlsArea::SIZE))
        .ok_or(FirmwareLoadError::AllocFloorOverflow { alloc_floor })?;
    let alloc_base = narrow_u32("alloc_base", tls.end())?;
    Ok(MemoryPlacement {
        alloc_floor,
        tls,
        alloc_base,
        user_region_end,
        prx_region_end,
    })
}

#[cfg(test)]
#[path = "tests/firmware_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/tls_placement_tests.rs"]
mod tls_placement_tests;
