//! The guest address space, and the title ELF loaded into it.

use std::time::{Duration, Instant};

use cellgov_ps3_abi::process_address_space::{
    PS3_CHILD_STACKS_BASE, PS3_CHILD_STACKS_SIZE, PS3_PRIMARY_STACK_BASE, PS3_PRIMARY_STACK_SIZE,
    PS3_RSX_BASE, PS3_RSX_IOMAP_BASE, PS3_RSX_IOMAP_SIZE, PS3_RSX_SIZE, PS3_SPU_RESERVED_BASE,
    PS3_SPU_RESERVED_SIZE,
};

use super::types::{check_strict_reserved_vs_rsx_mirror, u32_or_die, PrepareOptions};
use crate::cli::env::parse_env_bool;
use crate::cli::exit::die;

/// Bump-arena base for HLE-side allocations, above the TLS scratch
/// at `0x10400000`.
pub const HLE_HEAP_BASE: u32 = 0x10410000;

/// The title image resident in guest memory, and the floor later
/// stages place code above.
pub(super) struct LoadedImage {
    pub mem: cellgov_mem::GuestMemory,
    pub state: cellgov_ppu::state::PpuState,
    /// Guest address of the title's OPD entry descriptor.
    pub entry: u64,
    pub mem_size: usize,
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
pub(super) fn load_image(
    opts: &PrepareOptions<'_>,
    elf_data: &[u8],
    t_start: Instant,
) -> LoadedImage {
    let required_size = cellgov_ppu::loader::required_memory_size(elf_data)
        .unwrap_or_else(|e| die(&format!("failed to parse ELF: {e:?}")));

    // 64KB alignment plus 2 MiB headroom for PRX.
    let min_for_kernel = 0x4000_0000usize;
    let game_size = required_size
        .checked_add(0xFFFF)
        .map(|v| v & !0xFFFF)
        .and_then(|v| v.checked_add(0x200000))
        .unwrap_or_else(|| {
            die(&format!(
                "required_size=0x{required_size:x} overflows usize"
            ))
        });
    let mem_size = game_size.max(min_for_kernel);
    if parse_env_bool("CELLGOV_BOOT_TRACE_MEM") {
        eprintln!(
            "boot: required_size=0x{required_size:x} game_size=0x{game_size:x} \
             floor=0x{min_for_kernel:x} mem_size=0x{mem_size:x} ({:.2} GiB)",
            mem_size as f64 / (1024.0 * 1024.0 * 1024.0),
        );
    }
    let mut state = cellgov_ppu::state::PpuState::new();
    if let Err(err) =
        check_strict_reserved_vs_rsx_mirror(opts.strict_reserved, opts.title.rsx_mirror())
    {
        die(&err.to_string());
    }
    let reserved_access = if opts.strict_reserved {
        cellgov_mem::RegionAccess::ReservedStrict
    } else {
        cellgov_mem::RegionAccess::ReservedZeroReadable
    };
    let rsx_access = if opts.strict_reserved {
        reserved_access
    } else if opts.title.rsx_mirror() {
        cellgov_mem::RegionAccess::ReadWrite
    } else {
        reserved_access
    };
    // Main must end at or below PS3_RSX_IOMAP_BASE so the iomap
    // region the title later writes through stays disjoint.
    if mem_size as u64 > PS3_RSX_IOMAP_BASE {
        die(&format!(
            "boot: required_size 0x{required_size:x} requires main mem_size \
             0x{mem_size:x} which exceeds PS3_RSX_IOMAP_BASE 0x{PS3_RSX_IOMAP_BASE:x}"
        ));
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
    .unwrap_or_else(|e| die(&format!("failed to build guest memory layout: {e:?}")));

    let t_mem_alloc = t_start.elapsed();

    let load_result = cellgov_ppu::loader::load_ppu_elf(elf_data, &mut mem, &mut state)
        .unwrap_or_else(|e| die(&format!("failed to load ELF: {e:?}")));
    let t_elf_load = t_start.elapsed();

    if opts.prescan {
        emit_prescan_report(elf_data, opts.elf_path);
    }

    let code_floor = {
        let rounded = required_size.checked_add(0xFFF).unwrap_or_else(|| {
            die(&format!(
                "required_size=0x{required_size:x} + 0xFFF overflows usize"
            ))
        }) & !0xFFF;
        u32_or_die("code_floor", rounded as u64)
    };

    LoadedImage {
        mem,
        state,
        entry: load_result.entry,
        mem_size,
        code_floor,
        t_mem_alloc,
        t_elf_load,
    }
}

/// Walk the title ELF's executable PT_LOAD segments through the
/// PPU decoder and print the gap report. Firmware PRX text is out
/// of scope; gaps there surface at execution time.
fn emit_prescan_report(elf_data: &[u8], elf_path: &str) {
    let (report, coverage) = match cellgov_ppu::prescan::scan_elf_text(elf_data) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("prescan: skipped, {e} in {elf_path}");
            return;
        }
    };
    for line in crate::game::prescan_format::format_prescan_report(&report, &coverage, elf_path) {
        eprintln!("{line}");
    }
}
