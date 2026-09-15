//! `run_module_start`: drive a PRX's module_start to completion or
//! fault on the title's `Runtime`. A decode-error at PC=0 with LR=0
//! at fault time is the clean-return sentinel.

use std::collections::BTreeMap;
use std::time::Instant;

use cellgov_core::{AddressSpaceId, Runtime, StepError};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;

use crate::cli::exit::die;
use crate::game::diag::{
    append_pc_ring_with_decode, append_syscall_ring, fetch_raw_at, format_fault,
};
use crate::game::step_loop::tty::{classify_tty_capture, TtyCaptureDecision};
use crate::game::step_loop::{PcRing, SyscallRing};

use super::tls::TLS_BASE;
use super::types::PrxLoadInfo;

/// Per-module step cap inside the module_start loop.
pub(in crate::game) const PER_MODULE_STEP_BUDGET: usize = 1_000_000;

/// Outcome of [`run_module_start`] on a single PRX.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::game) enum ModuleStartOutcome {
    /// PRX has no `module_start` OPD; nothing was run.
    Skipped,
    /// `module_start` returned via the LR=0 decode-error sentinel after
    /// `steps` `rt.step()` calls.
    Completed { steps: usize },
    /// `module_start` is in [`HLE_STUBBED_MODULE_STARTS`]; treated as
    /// CELL_OK without running.
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
pub(in crate::game) enum ModuleStartError {
    /// Per-module step cap reached without the LR=0 return sentinel.
    /// `detail` carries the same rings as [`Self::Stalled`]: a spin
    /// names the dependency it polled through its syscall ring.
    #[error(
        "module_start: {module} did not return within {budget} steps \
         (last pc=0x{last_pc:016x}); init likely spun on a missing dependency{detail}"
    )]
    Incomplete {
        module: String,
        budget: usize,
        last_pc: u64,
        detail: String,
    },
    /// The unit faulted at something other than the LR=0 sentinel.
    #[error("module_start: {module} faulted after {steps} steps:\n{detail}")]
    Faulted {
        module: String,
        steps: usize,
        detail: String,
    },
    /// `commit_step` rejected the step's effects.
    #[error("module_start: {module} commit_step failed at step {steps}: {detail}")]
    Commit {
        module: String,
        steps: usize,
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
        module: String,
        steps: usize,
        reason: String,
        detail: String,
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
#[derive(Debug, Clone)]
pub(in crate::game) struct ModuleStartEnv {
    /// Address space the module was loaded into.
    pub(in crate::game) space: AddressSpaceId,
    /// Unit whose PPU thread the transient unit dispatches as: the
    /// boot primary, or a spawned child's primary.
    pub(in crate::game) thread_owner: UnitId,
    /// Pid the transient unit is bound to; `None` for the boot
    /// process, whose units stay unbound.
    pub(in crate::game) pid: Option<u32>,
    /// Synthetic kernel-context OPD in `space` (r11 / r12).
    pub(in crate::game) kctx_opd: u64,
    /// Initial r1, inside a stack region of `space`.
    pub(in crate::game) stack_pointer: u64,
    /// `--dump-at-pc` / `--dump-skip`: the transient unit faults with
    /// a register dump when it reaches the PC, like a title unit.
    pub(in crate::game) break_pc: Option<(u64, u32)>,
    /// `--dump-mem-fault`: guest ranges hex-dumped alongside the
    /// register dump when the transient unit faults or breaks.
    pub(in crate::game) dump_mem_fault_ranges: Vec<(u64, u64)>,
}

/// Guest address of the thread-id word liblv2 reads back through
/// `r13 - 0x7030` for the module_start units of every process.
pub(in crate::game) const MODULE_START_TLS_THREAD_ID_ADDR: u64 = TLS_BASE;

/// Write the id of the thread the transient unit dispatches as into
/// its TLS header, as LV2 does for every thread it starts.
///
/// liblv2's lwmutex fast path stores that word as the owner and its
/// unlock checks the owner against it (vsh `sys_lwmutex_lock` /
/// `sys_lwmutex_unlock`); a kernel-granted lock writes the owner as
/// the primary's `PpuThreadId`, which is what the header must read
/// back or the unlock is refused with `CELL_EPERM`.
fn seed_tls_thread_id(rt: &mut Runtime, env: &ModuleStartEnv) {
    let tid = rt
        .lv2_host()
        .ppu_thread_id_for_unit(env.thread_owner)
        .unwrap_or_else(|| {
            die(&format!(
                "module_start: unit {:?} has no PPU thread record to seed the TLS header from",
                env.thread_owner,
            ))
        })
        .raw();
    let range = ByteRange::new(GuestAddr::new(MODULE_START_TLS_THREAD_ID_ADDR), 8)
        .expect("the TLS header word is a fixed in-range address");
    rt.place_bytes(env.space, range, &tid.to_be_bytes())
        .unwrap_or_else(|e| {
            die(&format!(
                "module_start: TLS thread-id seed at 0x{MODULE_START_TLS_THREAD_ID_ADDR:x} \
                 FAILED ({e:?})"
            ))
        });
}

fn space_memory(rt: &Runtime, space: AddressSpaceId) -> &GuestMemory {
    rt.space_memory(space).unwrap_or_else(|e| {
        die(&format!(
            "module_start: address space {} vanished mid-pass: {e}",
            space.raw()
        ))
    })
}

/// Drive a single PRX's `module_start` on the supplied `Runtime`.
///
/// Mutex / TLS state created here persists in the caller's
/// `Runtime` / `Lv2Host` / `GuestMemory` and is visible to every
/// later module_start and to the process `env` names.
pub(in crate::game) fn run_module_start(
    rt: &mut Runtime,
    prx_info: &PrxLoadInfo,
    env: &ModuleStartEnv,
) -> Result<ModuleStartOutcome, ModuleStartError> {
    let ms = match prx_info.module_start {
        Some(opd) => opd,
        None => {
            println!(
                "module_start: {} has no module_start, skipping",
                prx_info.name
            );
            return Ok(ModuleStartOutcome::Skipped);
        }
    };

    if HLE_STUBBED_MODULE_STARTS.contains(&prx_info.name.as_str()) {
        if crate::cli::env::parse_env_bool("CELLGOV_DISABLE_MODULE_START_HLE_STUBS") {
            println!(
                "module_start: {} HLE-stub DISABLED via env; running the honest \
                 LLE path (expected to stall at the producer-fed wait)",
                prx_info.name,
            );
        } else {
            println!(
                "module_start: {} HLE-stubbed to CELL_OK (declared LLE divergence: \
                 init waits on a producer-fed ring with no producer in CellGov)",
                prx_info.name,
            );
            return Ok(ModuleStartOutcome::HleStubbed);
        }
    }

    println!(
        "module_start: {} at pc=0x{:x} toc=0x{:x}",
        prx_info.name, ms.code, ms.toc,
    );

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
    seed_tls_thread_id(rt, env);

    let ms_unit_id = rt.register_unit_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = ms_state;
        if let Some((pc, skip)) = env.break_pc {
            unit.set_break_pc(pc, skip);
        }
        unit
    });
    // A child's module_start executes and faults in the child's
    // space; every memory consumer resolves through the unit's tag.
    if let Err(e) = rt.assign_unit_space(ms_unit_id, env.space) {
        die(&format!(
            "module_start: cannot tag {} (UnitId {ms_unit_id:?}) to address space {}: {e}",
            prx_info.name,
            env.space.raw(),
        ));
    }
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
        die(&format!(
            "module_start: aliasing {} (UnitId {ms_unit_id:?}) to the thread of unit {:?} \
             failed; that process's primary PPU thread was not seeded before its \
             module_start pass began",
            prx_info.name, env.thread_owner,
        ));
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
                        handle_module_start_tty(args, space_memory(rt, env.space));
                    }
                }

                if steps.is_multiple_of(10_000) {
                    let hle_total: usize = hle_calls.values().sum();
                    let lv2_total: usize = lv2_calls.values().sum();
                    println!(
                        "  module_start [{:>6}] {} distinct PCs, {} HLE / {} LV2 calls",
                        steps,
                        distinct_pcs.len(),
                        hle_total,
                        lv2_total,
                    );
                }

                if let Err(e) = rt.commit_step(&step.result, &step.effects) {
                    eprintln!("  module_start commit_step FAILED at step {steps}: {e:?}");
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
                        eprintln!(
                            "module_start: {}: unit {:?} (not the module_start unit \
                             {ms_unit_id:?}) faulted while the module_start was in \
                             flight; the pass continues\n{fault_text}",
                            prx_info.name, step.unit,
                        );
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
                    eprintln!("module_start {fault_text}");
                    let code_str = guest_code
                        .map(|c| format!("0x{c:08x}"))
                        .unwrap_or_else(|| format!("{fault:?}"));
                    let raw_str = match fetch_raw_at(space_memory(rt, env.space), fault_pc) {
                        Some(w) => format!("0x{w:08x}"),
                        None => "<unmapped>".to_string(),
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
    println!(
        "module_start: {} -- {} steps, {} distinct PCs, {:.1?}",
        outcome_label,
        steps,
        distinct_pcs.len(),
        elapsed,
    );

    if !hle_calls.is_empty() {
        println!("  module_start HLE calls:");
        let mut sorted: Vec<_> = hle_calls.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (idx, count) in sorted.iter().take(10) {
            println!("    {count:>8}x  hle_{idx}");
        }
    }
    if !lv2_calls.is_empty() {
        println!("  module_start LV2 syscalls:");
        let mut sorted: Vec<_> = lv2_calls.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (num, count) in sorted.iter().take(10) {
            println!("    {count:>8}x  syscall {num}");
        }
    }
    // Namespace witnesses, reported here as well as at bench close
    // because a module_start that stalls never reaches the step loop.
    // Counters are cumulative across every module_start so far. Field
    // order mirrors the bench-close line, which owns the grammar.
    let host = rt.lv2_host();
    let ipc = &host.observability().system_ipc_witness;
    if !ipc.is_silent() {
        eprintln!(
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
        );
        let inventory: Vec<String> = ipc
            .keys_touched
            .iter()
            .map(|(key, events)| format!("0x{key:016x}={events}"))
            .collect();
        eprintln!(
            "BENCH_SYSTEM_IPC_KEYS_AT_MODULE_START: {}",
            inventory.join(" ")
        );
    }
    // Seeded-ring stall witnesses; the producer-fed waits these count
    // are the declared cellSysutil module_start wall.
    if host.system_seed_applied(cellgov_ps3_abi::lv2::ipc::CELLSYSUTIL_SHM_IPC_KEY) {
        println!(
            "  module_start seed witnesses: ring_wakes={} cond0_producer_waits={} cond_signals={}",
            host.observability().cond_ring_wakes,
            host.observability().cond0_producer_waits(),
            host.observability().cond_signal_dispatches,
        );
        let by_slot: Vec<String> = host
            .observability()
            .cond0_producer_waits_by_slot
            .iter()
            .map(|(slot, n)| format!("slot{slot}={n}"))
            .collect();
        if !by_slot.is_empty() {
            println!(
                "  module_start cond0 producer waits by slot: {}",
                by_slot.join(" "),
            );
        }
        let keyed: Vec<String> = host
            .observability()
            .cond_keyed_signal_counts
            .iter()
            .map(|(key, n)| format!("{key:#018x}={n}"))
            .collect();
        if !keyed.is_empty() {
            println!("  module_start keyed cond signals: {}", keyed.join(" "));
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
        eprintln!(
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
        );
    }

    if !pc_hits.is_empty() {
        println!("  module_start top PCs by hit count:");
        let mut sorted: Vec<_> = pc_hits.iter().collect();
        sorted.sort_by(|&(pc_a, c_a), &(pc_b, c_b)| c_b.cmp(c_a).then(pc_a.cmp(pc_b)));
        let mem = space_memory(rt, env.space);
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
            println!("    {count:>10}x  PC=0x{:08x}  raw={raw}  {disasm}", **pc);
        }
    }

    // Drop the alias so post-boot syscall dispatch against this
    // retired UnitId hits the strict ESRCH path. The transient unit
    // itself stays in the registry as Faulted (scheduler skips it).
    // Install above died on failure, so a missing alias here means
    // the table changed underneath the loop.
    if !rt.lv2_host_mut().drop_ppu_thread_alias(ms_unit_id) {
        eprintln!(
            "module_start: INVARIANT: no alias to drop for {} (UnitId {ms_unit_id:?}); \
             post-boot strict-ESRCH behavior for this unit is unverified",
            prx_info.name,
        );
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
    // unit mid-execution and the caller dies on them, so asserting
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

fn handle_module_start_tty(args: &[u64; 9], mem: &cellgov_mem::GuestMemory) {
    match classify_tty_capture(args, mem) {
        TtyCaptureDecision::InBounds { bytes, .. } => {
            let preview = &bytes[..bytes.len().min(256)];
            let text = String::from_utf8_lossy(preview);
            print!("  module_start TTY: {text}");
        }
        TtyCaptureDecision::Oob { buf, len, reason } => {
            eprintln!("  module_start TTY dropped: buf=0x{buf:x}+0x{len:x}: {reason}");
        }
    }
}

#[cfg(test)]
#[path = "tests/module_start_tests.rs"]
mod tests;
