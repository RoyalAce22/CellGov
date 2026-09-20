//! The guest address space, and the title ELF loaded into it.

use std::time::{Duration, Instant};

use cellgov_ps3_abi::hw::address_space::{
    PS3_CHILD_STACKS_BASE, PS3_CHILD_STACKS_SIZE, PS3_PRIMARY_STACK_BASE, PS3_PRIMARY_STACK_SIZE,
    PS3_RSX_BASE, PS3_RSX_IOMAP_BASE, PS3_RSX_IOMAP_SIZE, PS3_RSX_SIZE, PS3_SPU_RESERVED_BASE,
    PS3_SPU_RESERVED_SIZE,
};

use super::types::{
    check_strict_reserved_vs_rsx_mirror, DiagnosticOptions, ExecutionOptions, TitleOptions,
};
use crate::env::EnvBoolError;
use crate::error::narrow_u32;
use crate::BootError;

/// Bump-arena base for HLE-side allocations, above the TLS scratch
/// at `0x10400000`.
pub const HLE_HEAP_BASE: u32 = 0x10410000;

/// Why the guest address space or the title image in it could not be
/// built.
#[derive(Debug, thiserror::Error)]
pub enum ImageError {
    /// The title ELF's program headers do not parse.
    #[error("failed to parse ELF: {source}")]
    ParseElf {
        /// The loader's own account of the refusal.
        source: cellgov_ppu::loader::LoadError,
    },
    /// The title ELF does not load into the region built for it.
    #[error("failed to load ELF: {source}")]
    LoadElf {
        /// The loader's own account of the refusal.
        source: cellgov_ppu::loader::LoadError,
    },
    /// The main region's size overflows `usize`.
    #[error("required_size=0x{required_size:x} overflows usize")]
    SizeOverflow {
        /// Bytes the image's PT_LOADs need.
        required_size: usize,
    },
    /// The code floor rounded up to a page overflows `usize`.
    #[error("required_size=0x{required_size:x} + 0xFFF overflows usize")]
    CodeFloorOverflow {
        /// Bytes the image's PT_LOADs need.
        required_size: usize,
    },
    /// The main region the image needs would overlap the RSX iomap
    /// region the title later writes through.
    #[error(
        "boot: required_size 0x{required_size:x} requires main mem_size 0x{mem_size:x} \
         which exceeds PS3_RSX_IOMAP_BASE 0x{iomap_base:x}"
    )]
    MainRegionOverlapsIomap {
        /// Bytes the image's PT_LOADs need.
        required_size: usize,
        /// The main region those bytes force.
        mem_size: usize,
        /// Where the RSX iomap region starts.
        iomap_base: u64,
    },
    /// The region list does not describe a valid address space.
    #[error("failed to build guest memory layout: {source}")]
    Layout {
        /// The address space's own account of the refusal.
        source: cellgov_mem::MemError,
    },
    /// A `CELLGOV_*` toggle this stage reads holds an unrecognized
    /// value.
    #[error("{0}")]
    EnvBool(#[from] EnvBoolError),
}

/// The title image resident in guest memory, and the floor later
/// stages place code above.
pub(super) struct LoadedImage {
    pub mem: cellgov_mem::GuestMemory,
    pub state: cellgov_ppu::state::PpuState,
    /// Guest address of the title's OPD entry descriptor.
    pub entry: u64,
    pub mem_size: usize,
    /// Highest address exclusive of the title ELF's PT_LOAD ranges.
    pub image_end: usize,
    /// First page past the title image: the floor the HLE trampolines
    /// and the firmware set are placed from.
    pub code_floor: u32,
    pub t_mem_alloc: Duration,
    pub t_elf_load: Duration,
}

/// Build the guest address space and load the title ELF into it.
///
/// The main region spans the user-memory region (`0x00010000`+) and
/// the EBOOT load region (`0x10000000`+) as one contiguous backing.
///
/// # Errors
///
/// The image does not parse or load, its size does not fit the fixed
/// layout, or `--strict-reserved` conflicts with the manifest.
pub(super) fn load_image(
    title: &TitleOptions<'_>,
    execution: &ExecutionOptions<'_>,
    diagnostics: &DiagnosticOptions<'_>,
    sink: &dyn crate::BootSink,
    elf_data: &[u8],
    t_start: Instant,
) -> Result<LoadedImage, BootError> {
    let required_size = cellgov_ppu::loader::required_memory_size(elf_data)
        .map_err(|source| ImageError::ParseElf { source })?;

    // 64KB alignment plus 2 MiB headroom for PRX.
    let min_for_kernel = 0x4000_0000usize;
    let game_size = required_size
        .checked_add(0xFFFF)
        .map(|v| v & !0xFFFF)
        .and_then(|v| v.checked_add(0x200000))
        .ok_or(ImageError::SizeOverflow { required_size })?;
    let mem_size = game_size.max(min_for_kernel);
    if crate::env::parse_bool("CELLGOV_BOOT_TRACE_MEM").map_err(ImageError::from)? {
        sink.warn(&format!(
            "boot: required_size=0x{required_size:x} game_size=0x{game_size:x} \
             floor=0x{min_for_kernel:x} mem_size=0x{mem_size:x} ({:.2} GiB)",
            mem_size as f64 / (1024.0 * 1024.0 * 1024.0),
        ));
    }
    let mut state = cellgov_ppu::state::PpuState::new();
    check_strict_reserved_vs_rsx_mirror(execution.strict_reserved, title.manifest.rsx_mirror())?;
    let reserved_access = if execution.strict_reserved {
        cellgov_mem::RegionAccess::ReservedStrict
    } else {
        cellgov_mem::RegionAccess::ReservedZeroReadable
    };
    let rsx_access = if execution.strict_reserved {
        reserved_access
    } else if title.manifest.rsx_mirror() {
        cellgov_mem::RegionAccess::ReadWrite
    } else {
        reserved_access
    };
    // Main must end at or below PS3_RSX_IOMAP_BASE so the iomap
    // region the title later writes through stays disjoint.
    if mem_size as u64 > PS3_RSX_IOMAP_BASE {
        return Err(ImageError::MainRegionOverlapsIomap {
            required_size,
            mem_size,
            iomap_base: PS3_RSX_IOMAP_BASE,
        }
        .into());
    }
    let mut mem = cellgov_mem::GuestMemory::from_regions(vec![
        cellgov_mem::Region::new(0, mem_size, "main", cellgov_mem::PageSize::Page64K),
        cellgov_mem::Region::new(
            PS3_RSX_IOMAP_BASE,
            PS3_RSX_IOMAP_SIZE,
            "rsx_iomap",
            cellgov_mem::PageSize::Page64K,
        ),
        cellgov_mem::Region::new(
            PS3_PRIMARY_STACK_BASE,
            PS3_PRIMARY_STACK_SIZE,
            "stack",
            cellgov_mem::PageSize::Page4K,
        ),
        cellgov_mem::Region::new(
            PS3_CHILD_STACKS_BASE,
            PS3_CHILD_STACKS_SIZE,
            "child_stacks",
            cellgov_mem::PageSize::Page4K,
        ),
        cellgov_mem::Region::with_access(
            PS3_RSX_BASE,
            PS3_RSX_SIZE,
            "rsx",
            cellgov_mem::PageSize::Page64K,
            rsx_access,
        ),
        cellgov_mem::Region::with_access(
            PS3_SPU_RESERVED_BASE,
            PS3_SPU_RESERVED_SIZE,
            "spu_reserved",
            cellgov_mem::PageSize::Page64K,
            reserved_access,
        ),
    ])
    .map_err(|source| ImageError::Layout { source })?;

    let t_mem_alloc = t_start.elapsed();

    let load_result = cellgov_ppu::loader::load_ppu_elf(elf_data, &mut mem, &mut state)
        .map_err(|source| ImageError::LoadElf { source })?;
    let t_elf_load = t_start.elapsed();

    if diagnostics.prescan {
        emit_prescan_report(elf_data, title.elf_path, sink);
    }

    let code_floor = {
        let rounded = required_size
            .checked_add(0xFFF)
            .ok_or(ImageError::CodeFloorOverflow { required_size })?
            & !0xFFF;
        narrow_u32("code_floor", rounded as u64)?
    };

    Ok(LoadedImage {
        mem,
        state,
        entry: load_result.entry,
        mem_size,
        image_end: required_size,
        code_floor,
        t_mem_alloc,
        t_elf_load,
    })
}

/// Walk the title ELF's executable PT_LOAD segments through the
/// PPU decoder and report the gaps. Firmware PRX text is out
/// of scope; gaps there surface at execution time.
fn emit_prescan_report(elf_data: &[u8], elf_path: &str, sink: &dyn crate::BootSink) {
    let (report, coverage) = match cellgov_ppu::prescan::scan_elf_text(elf_data) {
        Ok(pair) => pair,
        Err(e) => {
            sink.warn(&format!("prescan: skipped, {e} in {elf_path}"));
            return;
        }
    };
    for line in crate::prescan_format::format_prescan_report(&report, &coverage, elf_path) {
        sink.warn(&line);
    }
}
