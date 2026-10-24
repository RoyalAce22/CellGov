//! Running every loaded PRX's `module_start`.

use super::types::{BootServices, DiagnosticOptions};
use crate::prx::{run_module_start_batch, ModuleStartEnv, PrxLoadInfo, TlsArea};
use crate::BootError;
use cellgov_compare::BootOverrides;
use cellgov_core::Runtime;

/// The primary thread a `module_start` runs in: a `module_start` runs
/// in the primary thread, on its stack, before the title's entry does.
pub(super) struct PrimaryThread {
    /// The primary's unit; the transient units alias its `PpuThreadId`.
    pub unit: cellgov_event::UnitId,
    /// Its TLS area.
    pub tls: TlsArea,
    /// Its entry r1, where each transient unit's stack starts.
    pub entry_r1: u64,
}

/// How the `module_start` loop accounted for the modules it was given.
pub(super) struct ModuleStartCounts {
    /// Modules declaring a `module_start` entry.
    pub total: usize,
    pub started: usize,
    /// Modules whose `module_start` faulted; witnessed by
    /// `BENCH_MODULE_START_FAULTS`, and left un-started.
    pub faulted: usize,
    /// The `skip_module_start` boot override suppressed the loop, so
    /// `total` is not accounted for.
    pub skipped: bool,
}

/// Run each loaded PRX's `module_start` on a transient PPU unit
/// aliased to the primary's `PpuThreadId`.
///
/// The transient unit Faults at the LR=0 return sentinel; the alias is
/// dropped immediately so the retired `UnitId` no longer resolves to a
/// thread record.
///
/// # Errors
///
/// Every [`crate::prx::ModuleStartError`] but `Faulted`, which leaves the module
/// un-started and the boot alive.
pub(super) fn run_module_starts(
    rt: &mut Runtime,
    prx_modules: &[PrxLoadInfo],
    overrides: BootOverrides,
    diagnostics: &DiagnosticOptions<'_>,
    services: &BootServices,
    primary: PrimaryThread,
) -> Result<ModuleStartCounts, BootError> {
    let total = prx_modules
        .iter()
        .filter(|p| p.module_start.is_some())
        .count();
    let skipped = overrides.skip_module_start;
    let boot_env = ModuleStartEnv {
        space: cellgov_core::AddressSpaceId::BOOT,
        thread_owner: primary.unit,
        pid: None,
        tls: primary.tls,
        stack_pointer: primary.entry_r1,
        break_pc: diagnostics.dump_at_pc.map(|pc| (pc, diagnostics.dump_skip)),
        dump_mem_fault_ranges: diagnostics.dump_mem_fault_ranges.to_vec(),
        run_hle_stubbed: overrides.disable_module_start_hle_stubs,
        sink: std::rc::Rc::clone(&services.sink),
        ppu_tap: services.taps.ppu(),
    };
    let (started, faulted) = start_boot_modules(rt, prx_modules, &boot_env, skipped)?;

    // Override holds for the duration of the module_start loop: the
    // unit's own status is still Runnable, but effective_status must
    // read Blocked.
    debug_assert_eq!(
        rt.registry().effective_status(primary.unit),
        Some(cellgov_exec::UnitStatus::Blocked),
        "primary unit effective_status changed during module_start loop",
    );

    Ok(ModuleStartCounts {
        total,
        started,
        faulted,
        skipped,
    })
}

/// Run the boot's `module_start` batch unless the `skip_module_start`
/// override suppresses it. Returns the completed and faulted counts.
fn start_boot_modules(
    rt: &mut Runtime,
    prx_modules: &[PrxLoadInfo],
    env: &ModuleStartEnv,
    skipped: bool,
) -> Result<(usize, usize), BootError> {
    let sink = env.sink.as_ref();
    if skipped {
        if prx_modules.is_empty() {
            sink.warn(
                "module_start: boot override skip_module_start set, but no PRX was loaded -- \
                 it has no effect",
            );
        } else {
            sink.warn("module_start: skipped (boot override skip_module_start)");
        }
        return Ok((0, 0));
    }
    let (completed, faulted) = run_module_start_batch(rt, prx_modules, env)?;
    if !faulted.is_empty() {
        sink.warn(&format!(
            "BENCH_MODULE_START_FAULTS: count={} modules={}",
            faulted.len(),
            faulted.join(",")
        ));
    }
    Ok((completed, faulted.len()))
}
