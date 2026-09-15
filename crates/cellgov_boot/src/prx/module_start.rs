//! `run_module_start`: drive a PRX's module_start to completion or
//! fault on the title's `Runtime`. A decode-error at PC=0 with LR=0
//! at fault time is the clean-return sentinel.

use std::collections::BTreeMap;
use std::rc::Rc;
use std::time::Instant;

use cellgov_core::{AddressSpaceId, Runtime, StepError};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;

use crate::diag::{append_pc_ring_with_decode, append_syscall_ring, fetch_raw_at, format_fault};
use crate::step_loop::tty::{classify_tty_capture, TtyCaptureDecision};
use crate::step_loop::{PcRing, SyscallRing};
use crate::BootSink;

use super::tls::TLS_BASE;
use super::types::PrxLoadInfo;

/// Per-module step cap inside the module_start loop.
pub const PER_MODULE_STEP_BUDGET: usize = 1_000_000;

/// Outcome of [`run_module_start`] on a single PRX.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModuleStartOutcome {
    /// PRX has no `module_start` OPD; nothing was run.
    Skipped,
    /// `module_start` returned via the LR=0 decode-error sentinel after
    /// `steps` `rt.step()` calls.
    Completed {
        /// `rt.step()` calls the module took to return.
        steps: usize,
    },
    /// `module_start` is HLE-stubbed; treated as CELL_OK without
    /// running.
    HleStubbed,
}

/// module_start entries HLE-stubbed to CELL_OK -- declared LLE
/// divergences.
///
/// `cellSysutil_Library`: its init dispatcher terminally waits on a
/// producer-fed shared-memory ring; the producer lives outside
/// cellSysutil in real PS3 firmware and CellGov has no equivalent,
/// so the LLE path stalls AllBlocked.
const HLE_STUBBED_MODULE_STARTS: &[&str] = &["cellSysutil_Library"];

/// Why a module_start failed to complete.
#[derive(Debug, thiserror::Error)]
pub enum ModuleStartError {
    /// Per-module step cap reached without the LR=0 return sentinel.
    /// `detail` carries the same rings as [`Self::Stalled`]: a spin
    /// names the dependency it polled through its syscall ring.
    #[error(
        "module_start: {module} did not return within {budget} steps \
         (last pc=0x{last_pc:016x}); init likely spun on a missing dependency{detail}"
    )]
    Incomplete {
        /// The module whose init did not return.
        module: String,
        /// The per-module step cap it ran out of.
        budget: usize,
        /// The last PC it retired.
        last_pc: u64,
        /// Its last PCs and syscalls, already rendered.
        detail: String,
    },
    /// The unit faulted at something other than the LR=0 sentinel.
    #[error("module_start: {module} faulted after {steps} steps:\n{detail}")]
    Faulted {
        /// The module whose init faulted.
        module: String,
        /// Steps it took before faulting.
        steps: usize,
        /// The fault report, already rendered.
        detail: String,
    },
    /// `commit_step` rejected the step's effects.
    #[error("module_start: {module} commit_step failed at step {steps}: {detail}")]
    Commit {
        /// The module whose init was running.
        module: String,
        /// The step the commit was refused on.
        steps: usize,
        /// The commit pipeline's own account of the refusal.
        detail: String,
    },
    /// Runtime scheduler reached a non-runnable state (stall / max-
    /// steps from `rt.max_steps`) before the module returned.
    /// `detail` carries the last-PC and syscall rings so the stall
    /// names the wait site it parked at.
    #[error(
        "module_start: {module} stalled after {steps} steps ({reason}); \
         under unified runtime this is fail-fast{detail}"
    )]
    Stalled {
        /// The module whose init stalled.
        module: String,
        /// Steps it took before the scheduler stopped.
        steps: usize,
        /// Which scheduler state it stopped in.
        reason: String,
        /// Its last PCs and syscalls, already rendered.
        detail: String,
    },
    /// The owner unit holds no PPU thread record, so the TLS header has
    /// no thread id to seed.
    #[error("module_start: unit {owner:?} has no PPU thread record to seed the TLS header from")]
    NoThreadRecord {
        /// The unit whose thread the transient unit dispatches as.
        owner: UnitId,
    },
    /// The TLS thread-id seed write was refused.
    #[error("module_start: TLS thread-id seed at 0x{addr:016x} FAILED ({detail})")]
    TlsSeed {
        /// The TLS header word the seed targets.
        addr: u64,
        /// The commit pipeline's own account of the refusal.
        detail: String,
    },
    /// The address space the module was loaded into is gone.
    #[error("module_start: address space {space} vanished mid-pass: {detail}")]
    SpaceMissing {
        /// The address space the module was loaded into.
        space: u32,
        /// The runtime's own account of the miss.
        detail: String,
    },
    /// The transient unit could not be tagged to the module's space.
    #[error(
        "module_start: cannot tag {module} (UnitId {unit:?}) to address space {space}: {detail}"
    )]
    SpaceAssign {
        /// The module whose init was to run.
        module: String,
        /// The transient unit that could not be tagged.
        unit: UnitId,
        /// The address space it was to be tagged to.
        space: u32,
        /// The runtime's own account of the refusal.
        detail: String,
    },
    /// The transient unit could not alias to its process's primary
    /// thread, so its sync syscalls would not resolve a caller.
    #[error(
        "module_start: aliasing {module} (UnitId {unit:?}) to the thread of unit {owner:?} \
         failed; that process's primary PPU thread was not seeded before its module_start \
         pass began"
    )]
    AliasFailed {
        /// The module whose init was to run.
        module: String,
        /// The transient unit that could not alias.
        unit: UnitId,
        /// The unit whose thread it was to alias to.
        owner: UnitId,
    },
}

fn stall_detail(rt: &Runtime, pc_ring: &PcRing, sc_ring: &SyscallRing) -> String {
    let mut text = String::new();
    append_pc_ring_with_decode(&mut text, rt, pc_ring);
    append_syscall_ring(&mut text, sc_ring);
    text
}

/// Where a module_start runs: the process it belongs to and the
/// register seeds that differ between the boot process and a spawned
/// child.
#[derive(Clone)]
pub struct ModuleStartEnv {
    /// Address space the module was loaded into.
    pub space: AddressSpaceId,
    /// Unit whose PPU thread the transient unit dispatches as: the
    /// boot primary, or a spawned child's primary.
    pub thread_owner: UnitId,
    /// Pid the transient unit is bound to; `None` for the boot
    /// process, whose units stay unbound.
    pub pid: Option<u32>,
    /// Synthetic kernel-context OPD in `space` (r11 / r12).
    pub kctx_opd: u64,
    /// Initial r1, inside a stack region of `space`.
    pub stack_pointer: u64,
    /// `--dump-at-pc` / `--dump-skip`: the transient unit faults with
    /// a register dump when it reaches the PC, like a title unit.
    pub break_pc: Option<(u64, u32)>,
    /// `--dump-mem-fault`: guest ranges hex-dumped alongside the
    /// register dump when the transient unit faults or breaks.
    pub dump_mem_fault_ranges: Vec<(u64, u64)>,
    /// Run the LLE path of each HLE-stubbed `module_start` in place of its `CELL_OK` stub.
    pub run_hle_stubbed: bool,
    /// Where the pass reports each module it runs.
    pub sink: Rc<dyn BootSink>,
    /// The observer the transient unit reports its dispatches to.
    pub ppu_tap: Option<Rc<dyn cellgov_ppu::PpuTap>>,
}

/// Guest address of the thread-id word liblv2 reads back through
/// `r13 - 0x7030` for the module_start units of every process.
pub(crate) const MODULE_START_TLS_THREAD_ID_ADDR: u64 = TLS_BASE;

/// Write the id of the thread the transient unit dispatches as into
/// its TLS header, as LV2 does for every thread it starts.
///
/// liblv2's lwmutex fast path stores that word as the owner and its
/// unlock checks the owner against it (vsh `sys_lwmutex_lock` /
/// `sys_lwmutex_unlock`); a kernel-granted lock writes the owner as
/// the primary's `PpuThreadId`, which is what the header must read
/// back or the unlock is refused with `CELL_EPERM`.
fn seed_tls_thread_id(rt: &mut Runtime, env: &ModuleStartEnv) -> Result<(), ModuleStartError> {
    let tid = rt
        .lv2_host()
        .ppu_thread_id_for_unit(env.thread_owner)
        .ok_or(ModuleStartError::NoThreadRecord {
            owner: env.thread_owner,
        })?
        .raw();
    let range = ByteRange::new(GuestAddr::new(MODULE_START_TLS_THREAD_ID_ADDR), 8)
        .expect("invariant: the TLS header word is a fixed in-range address");
    rt.place_bytes(env.space, range, &tid.to_be_bytes())
        .map_err(|e| ModuleStartError::TlsSeed {
            addr: MODULE_START_TLS_THREAD_ID_ADDR,
            detail: format!("{e:?}"),
        })?;
    Ok(())
}

fn space_memory(rt: &Runtime, space: AddressSpaceId) -> Result<&GuestMemory, ModuleStartError> {
    rt.space_memory(space)
        .map_err(|e| ModuleStartError::SpaceMissing {
            space: space.raw(),
            detail: e.to_string(),
        })
}

/// Drive a single PRX's `module_start` on the supplied `Runtime`.
///
/// Mutex / TLS state created here persists in the caller's
/// `Runtime` / `Lv2Host` / `GuestMemory` and is visible to every
/// later module_start and to the process `env` names.
///
/// # Errors
///
/// Every [`ModuleStartError`]. Only [`ModuleStartError::Faulted`]
/// leaves the runtime fit to continue; the rest stop a unit
/// mid-execution.
pub fn run_module_start(
    rt: &mut Runtime,
    prx_info: &PrxLoadInfo,
    env: &ModuleStartEnv,
) -> Result<ModuleStartOutcome, ModuleStartError> {
    let sink = env.sink.as_ref();
    let ms = match prx_info.module_start {
        Some(opd) => opd,
        None => {
            sink.note(&format!(
                "module_start: {} has no module_start, skipping",
                prx_info.name
            ));
            return Ok(ModuleStartOutcome::Skipped);
        }
    };

    if HLE_STUBBED_MODULE_STARTS.contains(&prx_info.name.as_str()) {
        if env.run_hle_stubbed {
            sink.note(&format!(
                "module_start: {} HLE-stub DISABLED by boot override; running the honest \
                 LLE path (expected to stall at the producer-fed wait)",
                prx_info.name,
            ));
        } else {
            sink.note(&format!(
                "module_start: {} HLE-stubbed to CELL_OK (declared LLE divergence: \
                 init waits on a producer-fed ring with no producer in CellGov)",
                prx_info.name,
            ));
            return Ok(ModuleStartOutcome::HleStubbed);
        }
    }

    sink.note(&format!(
        "module_start: {} at pc=0x{:x} toc=0x{:x}",
        prx_info.name, ms.code, ms.toc,
    ));

    let mut ms_state = cellgov_ppu::state::PpuState::new();
    ms_state.pc = ms.code;
    ms_state.set_gpr(2, ms.toc);
    ms_state.set_gpr(1, env.stack_pointer);
    ms_state.set_gpr(11, env.kctx_opd);
    ms_state.set_gpr(12, env.kctx_opd);
    // PPC64 convention: r13 = TLS_area + 0x7030.
    ms_state.set_gpr(13, TLS_BASE + 0x30 + 0x7000);
    // LR=0 sentinel: blr from module_start jumps to PC=0, where the
    // all-zero word fails to decode and the fault signals a return.
    ms_state.set_lr(0);
    seed_tls_thread_id(rt, env)?;

    let ms_unit_id = rt.register_unit_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = ms_state;
        if let Some((pc, skip)) = env.break_pc {
            unit.set_break_pc(pc, skip);
        }
        if let Some(tap) = &env.ppu_tap {
            unit.set_tap(Rc::clone(tap));
        }
        unit
    });
    // A child's module_start executes and faults in the child's
    // space; every memory consumer resolves through the unit's tag.
    rt.assign_unit_space(ms_unit_id, env.space)
        .map_err(|e| ModuleStartError::SpaceAssign {
            module: prx_info.name.clone(),
            unit: ms_unit_id,
            space: env.space.raw(),
            detail: e.to_string(),
        })?;
    // Cross-module contract: the transient module_start unit shares
    // its process's primary-thread PpuThreadId so sync syscalls
    // resolve their caller. Real LV2 routes module_start through the
    // calling thread (sys_prx.cpp `_sys_prx_start_module`). Alias is
    // dropped after the unit retires so later lookups against this
    // UnitId fall through to the strict ESRCH path.
    if !rt
        .lv2_host_mut()
        .alias_unit_to_thread_of(ms_unit_id, env.thread_owner)
    {
        return Err(ModuleStartError::AliasFailed {
            module: prx_info.name.clone(),
            unit: ms_unit_id,
            owner: env.thread_owner,
        });
    }
    // Spawned-process units are bound to their pid so process-scoped
    // dispatch (getpid, exit sweeps) attributes them correctly.
    if let Some(pid) = env.pid {
        rt.lv2_host_mut().bind_unit_process(ms_unit_id, pid);
    }

    // Wall-clock display only, not ordering: never feeds
    // `sync_state_hash` or any scheduling decision.
    let t_start = Instant::now();
    let mut steps: usize = 0;
    let mut distinct_pcs = std::collections::BTreeSet::new();
    let mut hle_calls: BTreeMap<u32, usize> = BTreeMap::new();
    let mut lv2_calls: BTreeMap<u64, usize> = BTreeMap::new();
    let mut pc_hits: BTreeMap<u64, u64> = BTreeMap::new();
    let mut pc_ring = PcRing::new();
    let mut sc_ring = SyscallRing::new();
    let mut last_pc: u64 = ms.code;

    let result: Result<usize, ModuleStartError> = loop {
        if steps >= PER_MODULE_STEP_BUDGET {
            break Err(ModuleStartError::Incomplete {
                module: prx_info.name.clone(),
                budget: PER_MODULE_STEP_BUDGET,
                last_pc,
                detail: stall_detail(rt, &pc_ring, &sc_ring),
            });
        }
        match rt.step() {
            Ok(step) => {
                steps += 1;

                if let Some(pc) = step.result.local_diagnostics.pc {
                    last_pc = pc;
                    distinct_pcs.insert(pc);
                    *pc_hits.entry(pc).or_insert(0) += 1;
                    pc_ring.push((env.space, pc));
                }

                if let Some(args) = &step.result.syscall_args {
                    if args[0] >= 0x10000 {
                        let idx = (args[0] - 0x10000) as u32;
                        *hle_calls.entry(idx).or_insert(0) += 1;
                    } else {
                        *lv2_calls.entry(args[0]).or_insert(0) += 1;
                    }
                    let sc_pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    sc_ring.push((args[0], sc_pc));

                    if args[0] == cellgov_ps3_abi::lv2::syscall::TTY_WRITE {
                        // The unit is mid-flight, so this refusal leaves
                        // the loop through `break`: the alias drop and
                        // the witness lines below still run.
                        match space_memory(rt, env.space) {
                            Ok(mem) => handle_module_start_tty(args, mem, sink),
                            Err(e) => break Err(e),
                        }
                    }
                }

                if steps.is_multiple_of(10_000) {
                    let hle_total: usize = hle_calls.values().sum();
                    let lv2_total: usize = lv2_calls.values().sum();
                    sink.note(&format!(
                        "  module_start [{:>6}] {} distinct PCs, {} HLE / {} LV2 calls",
                        steps,
                        distinct_pcs.len(),
                        hle_total,
                        lv2_total,
                    ));
                }

                if let Err(e) = rt.commit_step(&step.result, &step.effects) {
                    sink.warn(&format!(
                        "  module_start commit_step FAILED at step {steps}: {e:?}"
                    ));
                    break Err(ModuleStartError::Commit {
                        module: prx_info.name.clone(),
                        steps,
                        detail: format!("{e:?}"),
                    });
                }

                if let Some(fault) = &step.result.fault {
                    let fault_pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    let guest_code = match fault {
                        cellgov_effects::FaultKind::Guest(c) => Some(*c),
                        _ => None,
                    };

                    // The scheduler runs every runnable unit here, so a
                    // thread this or an earlier module_start spawned
                    // (`sys_audio_Library` leaves two running) can fault
                    // while the transient unit waits on it. That thread
                    // stays Faulted in the registry like any title
                    // thread; the module_start itself is judged only by
                    // its own unit.
                    if step.unit != ms_unit_id {
                        let mut fault_text = format_fault(
                            rt,
                            step.unit,
                            &step.result,
                            fault,
                            steps,
                            &pc_ring,
                            &env.dump_mem_fault_ranges,
                        );
                        append_syscall_ring(&mut fault_text, &sc_ring);
                        sink.warn(&format!(
                            "module_start: {}: unit {:?} (not the module_start unit \
                             {ms_unit_id:?}) faulted while the module_start was in \
                             flight; the pass continues\n{fault_text}",
                            prx_info.name, step.unit,
                        ));
                        continue;
                    }

                    // LR=0 sentinel guards against a corrupted call
                    // target that happens to jump to PC=0 mid-run.
                    let lr_at_fault = step
                        .result
                        .local_diagnostics
                        .fault_regs
                        .as_ref()
                        .map(|r| r.lr)
                        .unwrap_or(u64::MAX);
                    if fault_pc == 0
                        && lr_at_fault == 0
                        && guest_code.is_some_and(cellgov_ppu::is_decode_error)
                    {
                        break Ok(steps);
                    }
                    let mut fault_text = format_fault(
                        rt,
                        ms_unit_id,
                        &step.result,
                        fault,
                        steps,
                        &pc_ring,
                        &env.dump_mem_fault_ranges,
                    );
                    append_syscall_ring(&mut fault_text, &sc_ring);
                    sink.warn(&format!("module_start {fault_text}"));
                    let code_str = guest_code
                        .map(|c| format!("0x{c:08x}"))
                        .unwrap_or_else(|| format!("{fault:?}"));
                    // The fault is the outcome; the word under it only
                    // decorates the report. So a space miss here goes
                    // in the detail and the refusal stays `Faulted`,
                    // the one refusal the callers continue past.
                    let raw_str = match space_memory(rt, env.space) {
                        Ok(mem) => match fetch_raw_at(mem, fault_pc) {
                            Some(w) => format!("0x{w:08x}"),
                            None => "<unmapped>".to_string(),
                        },
                        Err(e) => format!("<{e}>"),
                    };
                    break Err(ModuleStartError::Faulted {
                        module: prx_info.name.clone(),
                        steps,
                        detail: format!(
                            "{code_str} at pc=0x{fault_pc:x} (raw={raw_str})\n{fault_text}"
                        ),
                    });
                }
            }
            Err(e) => {
                let reason = match e {
                    StepError::NoRunnableUnit | StepError::AllBlocked => {
                        "NoRunnableUnit/AllBlocked".to_string()
                    }
                    StepError::MaxStepsExceeded => "MaxStepsExceeded (runtime cap)".to_string(),
                    other => format!("{other:?}"),
                };
                break Err(ModuleStartError::Stalled {
                    module: prx_info.name.clone(),
                    steps,
                    reason,
                    detail: stall_detail(rt, &pc_ring, &sc_ring),
                });
            }
        }
    };

    let elapsed = t_start.elapsed();
    let outcome_label = match &result {
        Ok(n) => format!("RETURNED after {n} steps"),
        Err(e) => format!("FAILED: {e}"),
    };
    sink.note(&format!(
        "module_start: {} -- {} steps, {} distinct PCs, {:.1?}",
        outcome_label,
        steps,
        distinct_pcs.len(),
        elapsed,
    ));

    if !hle_calls.is_empty() {
        sink.note("  module_start HLE calls:");
        let mut sorted: Vec<_> = hle_calls.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (idx, count) in sorted.iter().take(10) {
            sink.note(&format!("    {count:>8}x  hle_{idx}"));
        }
    }
    if !lv2_calls.is_empty() {
        sink.note("  module_start LV2 syscalls:");
        let mut sorted: Vec<_> = lv2_calls.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (num, count) in sorted.iter().take(10) {
            sink.note(&format!("    {count:>8}x  syscall {num}"));
        }
    }
    // Namespace witnesses, reported here as well as at bench close
    // because a module_start that stalls never reaches the step loop.
    // Counters are cumulative across every module_start so far. Field
    // order mirrors the bench-close line, which owns the grammar.
    let host = rt.lv2_host();
    let ipc = &host.observability().system_ipc_witness;
    if !ipc.is_silent() {
        sink.warn(&format!(
            "BENCH_SYSTEM_IPC_WITNESS_AT_MODULE_START: module={} shm_creates={} \
             shm_attaches={} shm_maps={} shm_writes={} cond_creates={} cond_waits={} \
             cond_signals={} event_queue_creates={} event_queue_references={} \
             event_queue_enqueues={} event_port_connects={} distinct_keys={}",
            prx_info.name,
            ipc.shm_creates,
            ipc.shm_attaches,
            ipc.shm_maps,
            ipc.shm_writes,
            ipc.cond_creates,
            ipc.cond_waits,
            ipc.cond_signals,
            ipc.event_queue_creates,
            ipc.event_queue_references,
            ipc.event_queue_enqueues,
            ipc.event_port_connects,
            ipc.keys_touched.len(),
        ));
        let inventory: Vec<String> = ipc
            .keys_touched
            .iter()
            .map(|(key, events)| format!("0x{key:016x}={events}"))
            .collect();
        sink.warn(&format!(
            "BENCH_SYSTEM_IPC_KEYS_AT_MODULE_START: {}",
            inventory.join(" ")
        ));
    }
    // Seeded-ring stall witnesses; the producer-fed waits these count
    // are the declared cellSysutil module_start wall.
    if host.system_seed_applied(cellgov_ps3_abi::lv2::ipc::CELLSYSUTIL_SHM_IPC_KEY) {
        sink.note(&format!(
            "  module_start seed witnesses: ring_wakes={} cond0_producer_waits={} cond_signals={}",
            host.observability().cond_ring_wakes,
            host.observability().cond0_producer_waits(),
            host.observability().cond_signal_dispatches,
        ));
        let by_slot: Vec<String> = host
            .observability()
            .cond0_producer_waits_by_slot
            .iter()
            .map(|(slot, n)| format!("slot{slot}={n}"))
            .collect();
        if !by_slot.is_empty() {
            sink.note(&format!(
                "  module_start cond0 producer waits by slot: {}",
                by_slot.join(" "),
            ));
        }
        let keyed: Vec<String> = host
            .observability()
            .cond_keyed_signal_counts
            .iter()
            .map(|(key, n)| format!("{key:#018x}={n}"))
            .collect();
        if !keyed.is_empty() {
            sink.note(&format!(
                "  module_start keyed cond signals: {}",
                keyed.join(" ")
            ));
        }
        // Parsed by the cellSysutil stall-signature tripwire; every
        // field must stay a machine-readable integer.
        let cond0_slot0_key = cellgov_ps3_abi::lv2::ipc::CELLSYSUTIL_COND0_IPC_KEY_BASE;
        let cond0_slot0_signals = host
            .observability()
            .cond_keyed_signal_counts
            .get(&cond0_slot0_key)
            .copied()
            .unwrap_or(0);
        let slot0_producer_waits = host
            .observability()
            .cond0_producer_waits_by_slot
            .get(&0)
            .copied()
            .unwrap_or(0);
        sink.warn(&format!(
            "BENCH_CELLSYSUTIL_SEED_WITNESS: module={} stalled={} steps={} ring_wakes={} \
             cond0_producer_waits={} slot0_producer_waits={} cond_signals={} \
             cond0_slot0_signals={}",
            prx_info.name,
            u8::from(result.is_err()),
            steps,
            host.observability().cond_ring_wakes,
            host.observability().cond0_producer_waits(),
            slot0_producer_waits,
            host.observability().cond_signal_dispatches,
            cond0_slot0_signals,
        ));
    }

    // `result` is already decided, so a space miss here costs the
    // disassembly and nothing else. The warn channel names it, and the
    // pass keeps the outcome it exists to report.
    if !pc_hits.is_empty() {
        match space_memory(rt, env.space) {
            Ok(mem) => {
                sink.note("  module_start top PCs by hit count:");
                let mut sorted: Vec<_> = pc_hits.iter().collect();
                sorted.sort_by(|&(pc_a, c_a), &(pc_b, c_b)| c_b.cmp(c_a).then(pc_a.cmp(pc_b)));
                for (pc, count) in sorted.iter().take(20) {
                    let (raw, disasm) = match fetch_raw_at(mem, **pc) {
                        Some(w) => (
                            format!("0x{w:08x}"),
                            cellgov_ppu::decode::decode(w)
                                .ok()
                                .map(|insn| <&'static str>::from(&insn).to_string())
                                .unwrap_or_else(|| "<baddec>".into()),
                        ),
                        None => ("<unmapped>".to_string(), "<unmapped>".to_string()),
                    };
                    sink.note(&format!(
                        "    {count:>10}x  PC=0x{:08x}  raw={raw}  {disasm}",
                        **pc
                    ));
                }
            }
            Err(e) => sink.warn(&format!(
                "  module_start top-PC disassembly unavailable for {}: {e}",
                prx_info.name,
            )),
        }
    }

    // Drop the alias so post-boot syscall dispatch against this
    // retired UnitId hits the strict ESRCH path. The transient unit
    // itself stays in the registry as Faulted (scheduler skips it).
    // Install above returned on failure, so a missing alias here means
    // the table changed underneath the loop.
    if !rt.lv2_host_mut().drop_ppu_thread_alias(ms_unit_id) {
        sink.warn(&format!(
            "module_start: INVARIANT: no alias to drop for {} (UnitId {ms_unit_id:?}); \
             post-boot strict-ESRCH behavior for this unit is unverified",
            prx_info.name,
        ));
    }

    // The transient unit must be retired before the loop moves on: a
    // runnable one would let the scheduler resume module_start code
    // after the module returned, under an alias that no longer exists.
    // Scoped to this unit, not to a registry-wide count -- a
    // `module_start` may legitimately leave PPU threads of its own
    // running (`sys_audio_Library` creates two), and those are the
    // guest's, not this unit.
    //
    // Only where the caller keeps going: a clean return, or a guest
    // fault it records and moves past. The remaining errors stop the
    // unit mid-execution and the caller raises them, so asserting
    // there would replace a named fail-fast message with an assertion
    // panic that says less.
    let caller_continues = matches!(result, Ok(_) | Err(ModuleStartError::Faulted { .. }));
    debug_assert!(
        !caller_continues
            || rt.registry().effective_status(ms_unit_id)
                != Some(cellgov_exec::UnitStatus::Runnable),
        "module_start {} left its own transient unit ({ms_unit_id:?}) runnable",
        prx_info.name,
    );

    result.map(|steps| ModuleStartOutcome::Completed { steps })
}

fn handle_module_start_tty(args: &[u64; 9], mem: &cellgov_mem::GuestMemory, sink: &dyn BootSink) {
    match classify_tty_capture(args, mem) {
        TtyCaptureDecision::InBounds { bytes, .. } => {
            let preview = &bytes[..bytes.len().min(256)];
            let text = String::from_utf8_lossy(preview);
            sink.guest_text(&format!("  module_start TTY: {text}"));
        }
        TtyCaptureDecision::Oob { buf, len, reason } => {
            sink.warn(&format!(
                "  module_start TTY dropped: buf=0x{buf:x}+0x{len:x}: {reason}"
            ));
        }
    }
}

#[cfg(test)]
#[path = "tests/module_start_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/module_start_override_tests.rs"]
mod override_tests;
