//! The firmware PRX set the title's import tables name, and the
//! heap floor its extent forces.

use std::time::{Duration, Instant};

use super::types::{BootServices, DiagnosticOptions, TitleOptions};
use crate::error::narrow_u32;
use crate::prx::{
    install_unresolved_trampolines_only, load_firmware_set_bound, FirmwareLoadError, HostLinkMaps,
    PrxLoadInfo, VerifiedFirmware, TLS_BASE,
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

pub(super) fn check_tls_reservation(
    title_image_end: usize,
    prx_modules: &[PrxLoadInfo],
) -> Result<(), FirmwareLoadError> {
    let image_end = prx_modules
        .iter()
        .map(|module| module.data_end)
        .max()
        .unwrap_or(0)
        .max(title_image_end as u64);
    if image_end > TLS_BASE {
        return Err(FirmwareLoadError::RegionSize {
            image_end,
            tls_base: TLS_BASE,
        });
    }
    Ok(())
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
        matches!(
            title.manifest.source,
            crate::manifest::GameSource::FirmwareExec { .. }
        ),
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
///
/// # Errors
///
/// Rounding the floor up to a 64 KB page overflows, or the rounded base
/// does not fit a `u32`.
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
    let alloc_base = {
        let rounded = alloc_floor
            .checked_add(0xFFFF)
            .ok_or(FirmwareLoadError::AllocFloorOverflow { alloc_floor })?
            & !0xFFFF;
        narrow_u32("alloc_base", rounded.max(0x0001_0000) as u64)?
    };
    Ok(MemoryPlacement {
        alloc_floor,
        alloc_base,
        user_region_end,
        prx_region_end,
    })
}

#[cfg(test)]
#[path = "tests/firmware_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/tls_reservation_tests.rs"]
mod tls_reservation_tests;
