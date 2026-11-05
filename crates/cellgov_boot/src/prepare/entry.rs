//! The title's primary entry state, the predecode shadow bounded to
//! executable code, and the primary unit's registration.

use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ppu::PpuExecutionUnit;

use super::firmware::MemoryPlacement;
use super::params::{primary_entry_sp, primary_stack_top, BootParams};
use super::types::{BootDebugOptions, DiagnosticOptions, ExecutionOptions, PpuShadow};
use crate::env::EnvBoolError;
use crate::error::narrow_u32;
use crate::guest_args::GuestArgsError;
use crate::prx::TlsArea;
use crate::BootError;

/// Where the title's primary thread starts: its entry OPD and its TLS
/// area.
#[derive(Debug, Clone, Copy)]
pub(super) struct ProcessStart {
    /// Guest address of the title's OPD entry descriptor.
    pub entry: u64,
    /// The main thread's TLS area.
    pub tls: TlsArea,
}

/// Seed the entry registers other than r1 and the arguments in r3..r5,
/// which the caller seeds first.
///
/// The `ppu_entry_state` and `ppu_entry_tls` console captures settle
/// each value, with and without a TLS segment in the image. LR and r7
/// on the console are addresses in the code that LV2 maps above the
/// image and that calls the entry. CellGov does not run that code, so
/// it leaves these 0:
/// - LR, the return site in that code;
/// - r7, the termination function it passes;
/// - r10, which no document names. It is below the image's top
///   segment and differs between the two captures.
fn seed_process_start_registers(
    state: &mut cellgov_ppu::state::PpuState,
    malloc_pagesize: u32,
    start: ProcessStart,
) {
    state.set_lr(0);
    state.set_gpr(6, 0);
    // [CBE-Handbook p:396 s:14.3.1.2] R7 is the termination-function pointer; zero asks for no action.
    state.set_gpr(7, 0);
    state.set_gpr(8, start.entry);
    state.set_gpr(9, 0);
    state.set_gpr(10, 0);
    state.set_gpr(11, cellgov_ps3_abi::lv2::process::PRIMARY_ENTRY_R11);
    state.set_gpr(12, malloc_pagesize as u64);
    state.set_cr(cellgov_ps3_abi::lv2::process::PRIMARY_ENTRY_CR);
    // The code that calls the entry branches through CTR, from r0.
    state.set_gpr(0, state.pc);
    state.set_ctr(state.pc);
    // The code that calls the entry keeps argv, argc and envp in these.
    state.set_gpr(28, state.gpr[4]);
    state.set_gpr(29, state.gpr[3]);
    state.set_gpr(31, state.gpr[5]);
    // r13 is the PS3 PPC64 ABI TLS pointer; LV2 seeds it at process
    // creation and sys_initialize_tls does not touch it.
    state.set_gpr(13, start.tls.thread_pointer());
}

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
    /// The title's program headers, which bound the predecode shadow,
    /// did not read.
    #[error("predecode shadow: the title's program headers: {source}")]
    ProgramHeaders {
        /// The loader's refusal.
        #[source]
        source: cellgov_ppu::loader::LoadError,
    },
}

/// Stamp the title's primary entry state into `state`, and place the
/// argument block: the caller's guest args, or the one argument the
/// console passes.
///
/// The registers follow the console's process start; the args-block
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
    execution: &ExecutionOptions<'_>,
    diagnostics: &DiagnosticOptions<'_>,
    sink: &dyn crate::BootSink,
    params: &BootParams,
    start: ProcessStart,
) -> Result<(), EntryError> {
    let (block, r1) = entry_args(execution.guest_args, params.primary_stack_size)?;
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
    if diagnostics.print_banner && !execution.guest_args.is_empty() {
        sink.note(&format!(
            "guest args: argc={} argv=0x{:08x} r1=0x{r1:08x}",
            block.argc, block.argv_addr
        ));
    }
    state.set_gpr(1, r1);
    state.set_gpr(3, block.argc);
    state.set_gpr(4, block.argv_addr);
    state.set_gpr(5, block.envp_addr);
    seed_process_start_registers(state, params.malloc_pagesize, start);
    Ok(())
}

/// The argument block at the top of the primary stack, and the entry r1.
///
/// With no guest args, the block holds [`crate::guest_args::TITLE_ARGV0`].
/// The entry r1 is [`primary_entry_sp`] when the block clears it. A
/// larger block moves the entry r1 below the block.
///
/// # Errors
///
/// Returns [`GuestArgsError`] if a guest arg holds a NUL or the block
/// does not fit the stack.
fn entry_args(
    guest_args: &[String],
    stack_size: u32,
) -> Result<(crate::guest_args::GuestArgsBlock, u64), GuestArgsError> {
    let console_args = [crate::guest_args::TITLE_ARGV0.to_string()];
    let args = if guest_args.is_empty() {
        &console_args[..]
    } else {
        guest_args
    };
    let block = crate::guest_args::build_args_block(
        cellgov_ps3_abi::hw::address_space::PS3_PRIMARY_STACK_START,
        primary_stack_top(stack_size),
        args,
    )?;
    let r1 = block.initial_r1.min(primary_entry_sp(stack_size));
    Ok((block, r1))
}

/// Where the predecode shadow ends: past the title's last executable
/// segment and `prx_region_end`, the end of the firmware set's region,
/// which holds the trampolines too.
///
/// A title's data segment can sit far above its text, and the shadow
/// leaves that span out. `alloc_floor` caps the end, since the heap
/// begins there, and so does `memory_len`. A fetch outside the shadow
/// decodes on demand.
///
/// # Errors
///
/// The title's program headers do not read.
fn shadow_extent(
    elf_data: &[u8],
    prx_region_end: usize,
    alloc_floor: usize,
    memory_len: usize,
) -> Result<usize, EntryError> {
    let title_code_end = cellgov_ppu::loader::pt_load_segments(elf_data)
        .map_err(|source| EntryError::ProgramHeaders { source })?
        .iter()
        .filter(|segment| segment.executable)
        .map(|segment| segment.vaddr.saturating_add(segment.memsz))
        .max()
        .unwrap_or(0);
    let code_end = usize::try_from(title_code_end)
        .unwrap_or(usize::MAX)
        .max(prx_region_end);
    Ok(code_end.min(alloc_floor).min(memory_len))
}

/// Build the predecode shadow the primary unit registers with.
///
/// The shadow covers `[0, shadow_extent)` and runs the passes that
/// `passes` names. [`PpuShadow::Off`] gives `None`.
///
/// Built before `module_start`, which only mutates data segments
/// (mutex tables, allocator state), so the shadow still holds for the
/// title's first instruction; transient `module_start` units decode on
/// demand.
///
/// # Errors
///
/// `CELLGOV_RUNGAME_PROFILE` holds an unrecognized value, or the
/// title's program headers do not read.
pub(super) fn build_instruction_shadow(
    rt: &Runtime,
    elf_data: &[u8],
    placement: &MemoryPlacement,
    code_floor: u32,
    passes: PpuShadow,
    sink: &dyn crate::BootSink,
) -> Result<Option<PredecodedShadow>, EntryError> {
    let build = match passes {
        PpuShadow::Off => return Ok(None),
        PpuShadow::Quickened => PredecodedShadow::build_quickened,
        PpuShadow::Fused => PredecodedShadow::build,
    };
    let alloc_floor = placement.alloc_floor;
    let shadow_extent = shadow_extent(
        elf_data,
        placement.prx_region_end,
        alloc_floor,
        rt.memory().as_bytes().len(),
    )?;
    let t_shadow_start = std::time::Instant::now();
    let shadow = build(0, &rt.memory().as_bytes()[..shadow_extent]);
    if crate::env::parse_bool("CELLGOV_RUNGAME_PROFILE")? {
        let user_region_end = placement.user_region_end;
        let prx_region_end = placement.prx_region_end;
        sink.warn(&format!(
            "rungame_profile_shadow: {passes:?} shadow build over {shadow_extent} bytes took {:.2}ms \
             (alloc_floor=0x{alloc_floor:08x} user_region_end=0x{user_region_end:08x} \
             code_floor=0x{code_floor:08x} prx_region_end=0x{prx_region_end:08x})",
            t_shadow_start.elapsed().as_secs_f64() * 1000.0
        ));
    }
    Ok(Some(shadow))
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
    shadow: Option<PredecodedShadow>,
    debug_opts: BootDebugOptions,
    params: &BootParams,
    entry: u64,
) -> Result<cellgov_event::UnitId, BootError> {
    let primary_unit_id = rt.register_unit_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = state;
        if let Some(shadow) = shadow {
            unit.set_instruction_shadow(shadow);
        }
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

#[cfg(test)]
#[path = "tests/entry_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/shadow_choice_tests.rs"]
mod shadow_choice_tests;

#[cfg(test)]
#[path = "tests/entry_argv_console_tests.rs"]
mod entry_argv_console_tests;

#[cfg(test)]
#[path = "tests/entry_registers_console_tests.rs"]
mod entry_registers_console_tests;

#[cfg(test)]
#[path = "tests/entry_process_state_console_tests.rs"]
mod entry_process_state_console_tests;
