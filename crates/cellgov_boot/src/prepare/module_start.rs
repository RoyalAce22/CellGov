//! Running every loaded PRX's `module_start`.

use cellgov_core::Runtime;
use cellgov_ps3_abi::hw::address_space::PS3_PRIMARY_STACK_BASE;

use super::params::primary_entry_sp;
use super::types::PrepareOptions;
use crate::prx::{
    run_module_start, ModuleStartEnv, ModuleStartError, ModuleStartOutcome, PrxLoadInfo,
};
use crate::BootError;

/// Stack pointer the transient `module_start` units run on.
///
/// Sits inside the primary stack reservation but below the game's own
/// entry SP, so a `module_start` frame and the title's frames cannot
/// overlap.
const MODULE_START_STACK_POINTER: u64 = PS3_PRIMARY_STACK_BASE + 0x8000;

// `primary_entry_sp` owns the game side of the reservation, so the two
// addresses can move independently.
const _: () = assert!(MODULE_START_STACK_POINTER < primary_entry_sp());

/// How the `module_start` loop accounted for the modules it was given.
pub(super) struct ModuleStartCounts {
    /// Modules declaring a `module_start` entry.
    pub total: usize,
    pub started: usize,
    /// Modules whose `module_start` faulted; witnessed by
    /// `BENCH_MODULE_START_FAULTS`, and left un-started.
    pub faulted: usize,
    /// `CELLGOV_SKIP_MODULE_START` suppressed the loop, so `total` is
    /// not accounted for.
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
    opts: &PrepareOptions<'_>,
    primary_unit_id: cellgov_event::UnitId,
    kctx_opd: u64,
) -> Result<ModuleStartCounts, BootError> {
    let total = prx_modules
        .iter()
        .filter(|p| p.module_start.is_some())
        .count();
    let skipped =
        crate::env::parse_bool("CELLGOV_SKIP_MODULE_START").map_err(ModuleStartError::from)?;
    let boot_env = ModuleStartEnv {
        space: cellgov_core::AddressSpaceId::BOOT,
        thread_owner: primary_unit_id,
        pid: None,
        kctx_opd,
        stack_pointer: MODULE_START_STACK_POINTER,
        break_pc: opts.dump_at_pc.map(|pc| (pc, opts.dump_skip)),
        dump_mem_fault_ranges: opts.dump_mem_fault_ranges.to_vec(),
        sink: std::rc::Rc::clone(&opts.sink),
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
                opts.sink.warn(&format!(
                    "BENCH_MODULE_START_FAULTS: count={} modules={}",
                    faulted.len(),
                    faulted.join(",")
                ));
            }
            (completed, faulted.len())
        }
        (false, true) => {
            opts.sink
                .warn("module_start: skipped (CELLGOV_SKIP_MODULE_START set)");
            (0, 0)
        }
        (true, true) => {
            opts.sink.warn(
                "module_start: CELLGOV_SKIP_MODULE_START set, but no PRX was loaded -- flag has no effect"
            );
            (0, 0)
        }
        (true, false) => (0, 0),
    };

    // Override holds for the duration of the module_start loop: the
    // unit's own status is still Runnable, but effective_status must
    // read Blocked.
    debug_assert_eq!(
        rt.registry().effective_status(primary_unit_id),
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
