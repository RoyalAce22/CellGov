//! The title's primary entry state, the predecode shadow bounded to
//! executable code, and the primary unit's registration.

use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::address_space::PS3_PRIMARY_STACK_TOP;

use super::firmware::MemoryPlacement;
use super::params::{primary_entry_sp, BootParams};
use super::types::{BootDebugOptions, PrepareOptions};
use crate::env::EnvBoolError;
use crate::error::narrow_u32;
use crate::guest_args::GuestArgsError;
use crate::prx::TLS_BASE;
use crate::BootError;

/// Why the primary thread's entry state could not be seeded.
#[derive(Debug, thiserror::Error)]
pub enum EntryError {
    /// The guest argv block does not fit the primary stack.
    #[error("--guest-arg: {0}")]
    GuestArgs(#[from] GuestArgsError),
    /// The argv block's placement is not an addressable range.
    #[error("--guest-arg: args block 0x{base:08x}+0x{len:x} is not a valid range")]
    ArgsBlockBadRange {
        /// Guest address the block would start at.
        base: u64,
        /// The block's length in bytes.
        len: usize,
    },
    /// The commit pipeline refused the argv block's placement.
    #[error("--guest-arg: placing args block at 0x{base:08x} FAILED ({detail})")]
    ArgsBlockPlace {
        /// Guest address the block would start at.
        base: u64,
        /// The commit pipeline's own account of the refusal.
        detail: String,
    },
    /// A `CELLGOV_*` toggle this stage reads holds an unrecognized
    /// value.
    #[error("{0}")]
    EnvBool(#[from] EnvBoolError),
}

/// Stamp the title's primary entry state into `state`, and place the
/// guest args block when the caller supplied one.
///
/// r3..r10 follow the PS3 LV2 process-start convention; the args-block
/// layout lives in [`crate::guest_args`]. Called before the primary
/// unit is registered so `module_start` aliases bind to the real entry
/// state.
///
/// # Errors
///
/// The guest argv block does not fit the primary stack, or its
/// placement was refused; see [`EntryError`].
pub(super) fn seed_primary_entry_state(
    rt: &mut Runtime,
    state: &mut cellgov_ppu::state::PpuState,
    opts: &PrepareOptions<'_>,
    params: &BootParams,
    entry: u64,
) -> Result<(), EntryError> {
    let args_block = if opts.guest_args.is_empty() {
        None
    } else {
        let block = crate::guest_args::build_args_block(
            PS3_PRIMARY_STACK_TOP,
            params.primary_stack_size as u64,
            opts.guest_args,
        )?;
        let range = cellgov_mem::ByteRange::new(
            cellgov_mem::GuestAddr::new(block.base),
            block.bytes.len() as u64,
        )
        .ok_or(EntryError::ArgsBlockBadRange {
            base: block.base,
            len: block.bytes.len(),
        })?;
        rt.place_bytes(AddressSpaceId::BOOT, range, &block.bytes)
            .map_err(|e| EntryError::ArgsBlockPlace {
                base: block.base,
                detail: format!("{e:?}"),
            })?;
        if opts.print_banner {
            opts.sink.note(&format!(
                "guest args: argc={} argv=0x{:08x} r1=0x{:08x}",
                block.argc, block.argv_addr, block.initial_r1
            ));
        }
        Some(block)
    };
    match &args_block {
        Some(b) => {
            state.set_gpr(1, b.initial_r1);
            state.set_gpr(3, b.argc);
            state.set_gpr(4, b.argv_addr);
            state.set_gpr(5, b.envp_addr);
        }
        None => {
            state.set_gpr(1, primary_entry_sp());
            state.set_gpr(3, 0);
            state.set_gpr(4, 0);
            state.set_gpr(5, 0);
        }
    }
    state.set_lr(0);
    state.set_gpr(6, 0);
    state.set_gpr(7, 0x0100_0000);
    state.set_gpr(8, params.tls_info.map(|t| t.vaddr).unwrap_or(0));
    state.set_gpr(9, params.tls_info.map(|t| t.filesz).unwrap_or(0));
    state.set_gpr(10, params.tls_info.map(|t| t.memsz).unwrap_or(0));
    state.set_gpr(11, entry);
    state.set_gpr(12, params.malloc_pagesize as u64);
    // r13 is the PS3 PPC64 ABI TLS pointer; LV2 seeds it at process
    // creation and sys_initialize_tls does not touch it.
    state.set_gpr(13, TLS_BASE + 0x7030);
    Ok(())
}

/// Build the predecode shadow the primary unit registers with.
///
/// Bounded by [`MemoryPlacement::alloc_floor`], where code stops and
/// the heap begins. Built before `module_start`, which only mutates
/// data segments (mutex tables, allocator state), so the shadow still
/// holds for the title's first instruction; transient `module_start`
/// units decode on demand.
///
/// # Errors
///
/// `CELLGOV_RUNGAME_PROFILE` holds an unrecognized value.
pub(super) fn build_instruction_shadow(
    rt: &Runtime,
    placement: &MemoryPlacement,
    code_floor: u32,
    sink: &dyn crate::BootSink,
) -> Result<cellgov_ppu::shadow::PredecodedShadow, EntryError> {
    let alloc_floor = placement.alloc_floor;
    let shadow_extent = alloc_floor.min(rt.memory().as_bytes().len());
    let t_shadow_start = std::time::Instant::now();
    let shadow =
        cellgov_ppu::shadow::PredecodedShadow::build(0, &rt.memory().as_bytes()[..shadow_extent]);
    if crate::env::parse_bool("CELLGOV_RUNGAME_PROFILE")? {
        let user_region_end = placement.user_region_end;
        let prx_region_end = placement.prx_region_end;
        sink.warn(&format!(
            "rungame_profile_shadow: PredecodedShadow::build over {shadow_extent} bytes took {:.2}ms \
             (alloc_floor=0x{alloc_floor:08x} user_region_end=0x{user_region_end:08x} \
             code_floor=0x{code_floor:08x} prx_region_end=0x{prx_region_end:08x})",
            t_shadow_start.elapsed().as_secs_f64() * 1000.0
        ));
    }
    Ok(shadow)
}

/// Register the title's primary unit and seed its `PpuThreadId`.
///
/// Both happen before `module_start`s run: real LV2 attributes
/// `module_start` syscalls to the calling (primary) PPU thread, and
/// transient `module_start` units alias to this `PpuThreadId` for
/// caller resolution. The primary is marked non-runnable via the
/// registry status override so the scheduler skips it while
/// `module_start` units execute; the caller clears the override once
/// they are done.
///
/// # Errors
///
/// A stack base or TLS address too wide for the `u32` the LV2 thread
/// attributes carry it in.
pub(super) fn register_primary_unit(
    rt: &mut Runtime,
    state: cellgov_ppu::state::PpuState,
    shadow: cellgov_ppu::shadow::PredecodedShadow,
    debug_opts: BootDebugOptions,
    params: &BootParams,
    entry: u64,
) -> Result<cellgov_event::UnitId, BootError> {
    let primary_unit_id = rt.register_unit_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = state;
        unit.set_instruction_shadow(shadow);
        debug_opts.apply(&mut unit);
        unit
    });
    rt.set_unit_status_override(primary_unit_id, cellgov_exec::UnitStatus::Blocked);
    rt.lv2_host_mut().seed_primary_ppu_thread(
        primary_unit_id,
        cellgov_lv2::PpuThreadAttrs {
            entry,
            arg: 0,
            stack_base: narrow_u32("primary stack base", params.primary_stack_base)?,
            stack_size: params.primary_stack_size,
            priority: params.primary_prio,
            tls_base: params
                .tls_info
                .map(|t| narrow_u32("tls vaddr", t.vaddr))
                .transpose()?
                .unwrap_or(0),
        },
    );
    // Sync-syscall dispatch from aliased transient module_start
    // units resolves via the primary thread record.
    debug_assert!(
        rt.lv2_host()
            .ppu_thread_id_for_unit(primary_unit_id)
            .is_some(),
        "primary PPU thread record missing pre-module-start; alias targets would not resolve",
    );
    Ok(primary_unit_id)
}
