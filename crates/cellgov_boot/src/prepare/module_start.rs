//! Running every loaded PRX's `module_start`.

use super::types::{BootServices, DiagnosticOptions};
use crate::prx::{
    run_module_start, ModuleStartEnv, ModuleStartError, ModuleStartOutcome, PrxLoadInfo, TlsArea,
};
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
/// Every [`ModuleStartError`] but `Faulted`, which leaves the module
/// un-started and the boot alive.
pub(super) fn run_module_starts(
    rt: &mut Runtime,
    prx_modules: &[PrxLoadInfo],
    overrides: BootOverrides,
    diagnostics: &DiagnosticOptions<'_>,
    services: &BootServices,
    primary: PrimaryThread,
) -> Result<ModuleStartCounts, BootError> {
    let sink = services.sink();
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
    let (started, faulted) = match (prx_modules.is_empty(), skipped) {
        (false, false) => {
            let mut completed: usize = 0;
            let mut faulted: Vec<String> = Vec::new();
            for info in prx_modules {
                match run_module_start(rt, info, &boot_env) {
                    Ok(ModuleStartOutcome::Completed { .. })
                    | Ok(ModuleStartOutcome::HleStubbed) => completed += 1,
                    Ok(ModuleStartOutcome::Skipped) => {}
                    // A guest fault leaves the module un-started and the
                    // boot alive: the runner already tore the transient
                    // unit down (alias dropped, unit Faulted and
                    // skipped). The other error kinds can leave a parked
                    // unit behind, so they stay fatal.
                    Err(ModuleStartError::Faulted { module, .. }) => {
                        faulted.push(module);
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            if !faulted.is_empty() {
                sink.warn(&format!(
                    "BENCH_MODULE_START_FAULTS: count={} modules={}",
                    faulted.len(),
                    faulted.join(",")
                ));
            }
            (completed, faulted.len())
        }
        (false, true) => {
            sink.warn("module_start: skipped (boot override skip_module_start)");
            (0, 0)
        }
        (true, true) => {
            sink.warn(
                "module_start: boot override skip_module_start set, but no PRX was loaded -- \
                 it has no effect",
            );
            (0, 0)
        }
        (true, false) => (0, 0),
    };

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
