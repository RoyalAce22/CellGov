//! `boot bench` / `boot bench-once` machinery.
//!
//! A run set takes N subprocess measurements. It gates on what the
//! runs must reproduce exactly: steps, outcome, budget, and witness
//! map. It reports throughput separately, because elapsed time
//! measures the host as much as the emulator.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::{Duration, Instant};

use cellgov_compare::witness_parse::{parse_witness_lines, ParsedWitnesses};
use cellgov_compare::witnesses::{check_all, unrecorded};
use cellgov_compare::{
    BootOutcome, BootOutcomeParseError, BootSummary, FirmwareIdentity, GameIdentity, RunIdentity,
    RUN_IDENTITY_SENTINEL,
};
use cellgov_time::Budget;

use super::boot;
use super::manifest::{self, CellKey, TitleManifest};
use super::step_loop::bench_step_loop;
use crate::paths::{boot_anchor_path, workspace_root};

/// Subprocess measurements one `boot bench` invocation takes.
///
/// The determinism gate is exact at any count above one. The
/// throughput half fixes the count: min-of-N is the estimator there,
/// and two samples cannot outvote a single descheduling event.
pub const BENCH_DEFAULT_RUNS: usize = 3;

/// Cross-run wall spread above which a run set makes no throughput
/// claim, as a percentage of the fastest run.
///
/// A spread above the ceiling is a nonzero exit only under
/// `--strict-perf`. On a busy host the spread measures the host.
pub const BENCH_SPREAD_CEILING_PCT: f64 = 5.0;

/// The selection flags a run resolved its composition from.
///
/// The parent of a run set forwards these flags to every child, so
/// each process composes from the same store.
#[derive(Debug, Clone, Copy, Default)]
pub struct SelectionArgs<'a> {
    pub fw: Option<&'a str>,
    pub game_ver: Option<&'a str>,
    /// An unmanaged firmware tree, which bypasses the store.
    pub firmware_dir: Option<&'a str>,
    /// The root the CLI reads the store from.
    pub vfs_root: Option<&'a str>,
}

/// The cell a run is held against, and the two parameters the registry
/// fixes for it.
#[derive(Debug, Clone, Copy)]
pub struct AnchorPlan<'a> {
    /// `None` when the composition names no cell:
    ///
    /// - an unmanaged firmware tree carries no version;
    /// - a title the store does not hold has no game-version axis.
    pub cell: Option<&'a CellKey>,
    /// Instruction cap the cell's anchor is recorded under.
    pub max_steps: u64,
    /// Checkpoint the cell's anchor is recorded under.
    pub checkpoint: manifest::CheckpointTrigger,
}

/// Inputs common to every `boot bench` entry point.
#[derive(Debug, Clone, Copy)]
pub struct BenchOptions<'a> {
    pub title: &'a TitleManifest,
    pub elf_path: &'a str,
    pub max_steps: usize,
    /// What the registry declares for the cell this run composes.
    pub plan: AnchorPlan<'a>,
    /// The `sys/external` directory the firmware loader reads.
    pub firmware_dir: Option<&'a str>,
    pub composed_mounts: &'a [crate::composition::ComposedMount],
    /// The triple this run is measured against.
    pub identity: &'a cellgov_compare::RunIdentity,
    /// What the child re-resolves its own composition from.
    pub selection: SelectionArgs<'a>,
    pub strict_reserved: bool,
    pub checkpoint_override: Option<manifest::CheckpointTrigger>,
    pub budget_override: Option<Budget>,
    /// When true, scan the title ELF for unimplemented PPU
    /// encodings before execution and print the gap report.
    pub prescan: bool,
    /// Guest argv for the primary thread, `argv[0]` included. Empty
    /// keeps the no-args entry state (r3..r6 = 0).
    pub guest_args: &'a [String],
    /// Compare the run against the cell's committed anchor. Cleared
    /// by `--no-anchor-check` for the re-record workflow, where the
    /// anchor is expected to disagree.
    pub check_anchor: bool,
    /// Travels to the child, which reports it back on its
    /// `BENCH_RESULT` line.
    pub run_index: usize,
}

impl BenchOptions<'_> {
    /// Append the `boot bench-once` CLI form of this struct onto `cmd`.
    fn encode_to_command(&self, cmd: &mut std::process::Command) {
        cmd.arg("boot")
            .arg("bench-once")
            // The parent captures the child's streams and replays them
            // after exit, so a child bar would render into a pipe.
            .arg("--no-progress")
            .arg("--title")
            .arg(self.title.name())
            .arg("--max-steps")
            .arg(self.max_steps.to_string())
            .arg("--run-index")
            .arg(self.run_index.to_string());
        for (flag, value) in [
            ("--vfs-root", self.selection.vfs_root),
            ("--fw", self.selection.fw),
            ("--game-ver", self.selection.game_ver),
            ("--firmware-dir", self.selection.firmware_dir),
        ] {
            if let Some(v) = value {
                cmd.arg(flag).arg(v);
            }
        }
        if self.strict_reserved {
            cmd.arg("--strict-reserved");
        }
        if let Some(cp) = self.checkpoint_override {
            cmd.arg("--checkpoint").arg(cp.as_cli_str());
        }
        if let Some(b) = self.budget_override {
            cmd.arg("--budget").arg(b.raw().to_string());
        }
        if self.prescan {
            cmd.arg("--prescan");
        }
        for arg in self.guest_args {
            cmd.arg("--guest-arg").arg(arg);
        }
    }
}

/// One completed bench run.
#[derive(Debug, Clone, Copy)]
pub struct BenchBootResult {
    pub run_index: usize,
    pub steps: usize,
    pub wall: Duration,
    /// Instructions each step was granted; `steps * budget` is the
    /// count the run retired.
    pub budget: Budget,
    pub outcome: BootOutcome,
}

impl BenchBootResult {
    pub fn steps_per_sec(&self) -> f64 {
        let secs = self.wall.as_secs_f64();
        if secs == 0.0 {
            0.0
        } else {
            self.steps as f64 / secs
        }
    }
}

/// How the throughput half of a run set behaves.
#[derive(Debug, Clone, Copy)]
pub struct ThroughputPolicy {
    /// Subprocess measurements to take.
    pub runs: usize,
    /// Turn a throughput verdict the set could not reach into a
    /// nonzero exit. Set it only on an idle host.
    pub strict: bool,
}

/// What the throughput half of a run set concluded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ThroughputVerdict {
    /// Spread within [`BENCH_SPREAD_CEILING_PCT`]. `min` estimates the
    /// uncontended cost: contention only ever adds time, so the
    /// fastest run is the least contaminated one.
    Measured { min: Duration, spread_pct: f64 },
    /// Spread above the ceiling, so the set makes no throughput claim.
    Inconclusive { min: Duration, spread_pct: f64 },
    /// A run reported a zero wall, so there is no spread to compare.
    Unmeasurable,
}

impl ThroughputVerdict {
    fn is_measured(self) -> bool {
        matches!(self, Self::Measured { .. })
    }
}

/// Gate verdict for [`bench_boot_runs`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchGate {
    /// Every run reproduced the same steps, outcome and witness map,
    /// and the anchor comparison found nothing.
    Pass,
    /// Runs disagreed on retired step count, boot outcome, or a
    /// witness.
    DeterminismBreak,
    /// The run disagreed with the cell's committed anchor.
    AnchorDrift,
    /// The set reached no throughput claim under `--strict-perf`.
    SpreadExceeded,
}

/// Result of one [`bench_boot_runs`] invocation.
#[derive(Debug, Clone)]
pub struct BenchRunsOutcome {
    /// Every measurement taken, in the order they ran.
    pub runs: Vec<BenchBootResult>,
    pub throughput: ThroughputVerdict,
    pub gate: BenchGate,
    /// Every anchor disagreement found, empty unless `gate` is
    /// [`BenchGate::AnchorDrift`].
    pub anchor_failures: Vec<String>,
    /// Every way the runs failed to reproduce each other, empty unless
    /// `gate` is [`BenchGate::DeterminismBreak`].
    pub determinism_failures: Vec<String>,
}

/// Run one boot with the minimum step-loop bookkeeping needed to
/// detect termination.
///
/// RSX-init coupling: `set_rsx_mirror_writes` is driven by the
/// manifest's declared checkpoint, not the runtime-overridable
/// `checkpoint`, so a `--checkpoint pc=ADDR` override does not change
/// the boot trajectory's init path.
///
/// `trace_path` puts the runtime in `DeterminismCheck` mode, which
/// costs a state hash per step. No comparison reads a traced run's
/// wall time.
pub fn bench_boot(
    opts: BenchOptions<'_>,
    elf_data: Vec<u8>,
    authority_id: Option<u64>,
    control_flags1: Option<u32>,
    trace_path: Option<&str>,
    progress: &dyn crate::progress::ProgressSink,
) -> BenchBootResult {
    progress.phase(crate::progress::BootPhase::Loading.code());
    let prepared = boot::prepare(boot::PrepareOptions {
        title: opts.title,
        elf_path: opts.elf_path,
        elf_data,
        authority_id,
        control_flags1,
        firmware_dir: opts.firmware_dir,
        composed_mounts: opts.composed_mounts,
        identity: opts.identity,
        strict_reserved: opts.strict_reserved,
        dump_at_pc: None,
        dump_skip: 0,
        dump_mem_fault_ranges: &[],
        print_banner: false,
        runtime_max_steps: opts.max_steps,
        patch_bytes: &[],
        dump_mem_boot_addrs: &[],
        profile_pairs: false,
        budget_override: opts.budget_override,
        capture_state_trace: trace_path.is_some(),
        prescan: opts.prescan,
        guest_args: opts.guest_args,
    });
    let mut rt = prepared.rt;
    let authid_source = prepared.authid_source;
    let child_init = prepared.child_init;
    let step_budget = prepared.step_budget;
    let active_checkpoint = opts.checkpoint_override.unwrap_or(opts.plan.checkpoint);
    super::run::configure_rsx_from_manifest(&mut rt, opts.title);

    let mut steps: usize = 0;
    let t0 = Instant::now();
    // The denominator is `rt.max_steps()`, the cap on step() calls that
    // `resolve_boot_params` derived from the `--max-steps` instruction
    // cap. module_start already spent part of it.
    progress.totals(0, rt.max_steps() as u64);
    progress.preset_done(rt.steps_taken() as u64);
    progress.phase(crate::progress::BootPhase::Stepping.code());
    let outcome = bench_step_loop(
        &mut rt,
        active_checkpoint,
        &mut steps,
        &child_init,
        progress,
    );
    let wall = t0.elapsed();
    // Stop the bar before the witness block prints; see
    // `ProgressSink::finished`.
    progress.finished();

    // VRSAVE liveness witness: sum mfvrsave_executed across every
    // PPU unit so the integration gate can scrape it from stderr.
    let mut total_mfvrsave_executed: u64 = 0;
    let mut any_vrsave_written = false;
    for (_id, unit) in rt.registry().iter() {
        if let Some(ppu) = unit
            .as_any()
            .downcast_ref::<cellgov_ppu::PpuExecutionUnit>()
        {
            total_mfvrsave_executed =
                total_mfvrsave_executed.wrapping_add(ppu.state().mfvrsave_executed);
            if ppu.state().vrsave_written {
                any_vrsave_written = true;
            }
        }
    }
    eprintln!(
        "BENCH_VRSAVE_WITNESS: mfvrsave_executed={total_mfvrsave_executed} vrsave_written={any_vrsave_written}"
    );

    let host_invariant_breaks = rt.lv2_host().observability().invariant_break_count as u64;
    eprintln!("BENCH_HOST_INVARIANT_BREAKS_WITNESS: count={host_invariant_breaks}");
    // Only the first break of a boot prints its detail line, so the
    // per-site split is the only way to read the rest.
    let break_sites: Vec<String> = rt
        .lv2_host()
        .observability()
        .invariant_break_sites
        .iter()
        .map(|(site, hits)| format!("{site}={hits}"))
        .collect();
    if !break_sites.is_empty() {
        eprintln!(
            "BENCH_HOST_INVARIANT_BREAK_SITES: {}",
            break_sites.join(" ")
        );
    }

    // Per-mnemonic so word-width and doubleword paths report
    // independently.
    let mut ldarx_total: u64 = 0;
    let mut stdcx_total: u64 = 0;
    let mut lwarx_total: u64 = 0;
    let mut stwcx_total: u64 = 0;
    for (_id, unit) in rt.registry().iter() {
        if let Some(ppu) = unit
            .as_any()
            .downcast_ref::<cellgov_ppu::PpuExecutionUnit>()
        {
            ldarx_total = ldarx_total.wrapping_add(ppu.state().ldarx_executed);
            stdcx_total = stdcx_total.wrapping_add(ppu.state().stdcx_executed);
            lwarx_total = lwarx_total.wrapping_add(ppu.state().lwarx_executed);
            stwcx_total = stwcx_total.wrapping_add(ppu.state().stwcx_executed);
        }
    }
    eprintln!(
        "BENCH_ATOMIC_WITNESS: ldarx={ldarx_total} stdcx={stdcx_total} lwarx={lwarx_total} stwcx={stwcx_total}"
    );

    // MemFault witness: arm_entries counts entries to
    // ExecuteVerdict::MemFault; unmapped_routed increments only
    // inside the MemError::Unmapped arm.
    let mut mem_fault_arm_entries: u64 = 0;
    let mut mem_fault_unmapped_routed: u64 = 0;
    for (_id, unit) in rt.registry().iter() {
        if let Some(ppu) = unit
            .as_any()
            .downcast_ref::<cellgov_ppu::PpuExecutionUnit>()
        {
            mem_fault_arm_entries =
                mem_fault_arm_entries.wrapping_add(ppu.state().mem_fault_arm_entries);
            mem_fault_unmapped_routed =
                mem_fault_unmapped_routed.wrapping_add(ppu.state().mem_fault_unmapped_routed);
        }
    }
    eprintln!(
        "BENCH_MEM_FAULT_WITNESS: arm_entries={mem_fault_arm_entries} unmapped_routed={mem_fault_unmapped_routed}"
    );

    // Timer sleeps bypass Lv2Host::dispatch, so no other witness
    // records them; a guest sleep loop is invisible without this line.
    let timer_sleeps = rt.timer_sleep_dispatches();
    eprintln!("BENCH_TIMER_SLEEP_WITNESS: count={timer_sleeps}");

    let rsx_label_writes_committed = rt.rsx_label_writes_committed();
    eprintln!("BENCH_RSX_LABEL_WRITES_WITNESS: count={rsx_label_writes_committed}");

    let rsx_set_reference_dispatches = rt.rsx_set_reference_dispatches();
    eprintln!("BENCH_RSX_SET_REFERENCE_WITNESS: count={rsx_set_reference_dispatches}");

    let mut dcbz_total: u64 = 0;
    for (_id, unit) in rt.registry().iter() {
        if let Some(ppu) = unit
            .as_any()
            .downcast_ref::<cellgov_ppu::PpuExecutionUnit>()
        {
            dcbz_total = dcbz_total.wrapping_add(ppu.state().dcbz_executed);
        }
    }
    eprintln!("BENCH_DCBZ_WITNESS: count={dcbz_total}");

    let spu_image_registers = rt.lv2_host().content_store().register_invocations();
    eprintln!("BENCH_SPU_IMAGE_REGISTER_WITNESS: count={spu_image_registers}");

    let spu_thread_init_dispatches = rt
        .lv2_host()
        .observability()
        .spu_thread_initialize_dispatches;
    eprintln!("BENCH_SPU_THREAD_INIT_WITNESS: count={spu_thread_init_dispatches}");

    let lwmutex_acquires = rt.lv2_host().lwmutexes().acquires_count();
    let lwmutex_releases = rt.lv2_host().lwmutexes().releases_count();
    let cond_reacquires = rt.lv2_host().observability().cond_reacquire_wake_calls;
    eprintln!(
        "BENCH_LWMUTEX_COND_WITNESS: lwmutex_acquires={lwmutex_acquires} lwmutex_releases={lwmutex_releases} cond_reacquires={cond_reacquires}"
    );

    // lwmutex_unknown_locks is the cellSysmodule LoadModule-failure
    // signature; a wrong system authid reintroduces it.
    let program_authority_id = rt.lv2_host().program_authority_id();
    let lwmutex_unknown_locks = rt.lv2_host().observability().lwmutex_unknown_lock_count;
    eprintln!(
        "BENCH_AUTHORITY_ID_WITNESS: program_authority_id=0x{program_authority_id:016x} authid_source={authid_source} lwmutex_unknown_locks={lwmutex_unknown_locks}"
    );
    let mutex_unlock_not_owner = rt.lv2_host().observability().mutex_unlock_not_owner_count;
    eprintln!("BENCH_MUTEX_UNLOCK_WITNESS: not_owner={mutex_unlock_not_owner}");
    // Every non-zero immediate return, error or value. A code absent
    // here was never produced by LV2 this boot. Line convention:
    // inventory lines (a rendered map, like this one) suppress when
    // the map is empty; scalar count lines always print because
    // their zero is the finding; a line carrying both, like the
    // unsupported-syscall one, keeps its scalar and drops the tail.
    let codes: Vec<String> = rt
        .lv2_host()
        .observability()
        .dispatch_nonzero_returns
        .iter()
        .map(|(code, hits)| format!("0x{code:08x}={hits}"))
        .collect();
    if !codes.is_empty() {
        eprintln!("BENCH_DISPATCH_RETURN_WITNESS: {}", codes.join(" "));
    }
    // Same codes attributed to the arm that returned them.
    let pairs: Vec<String> = rt
        .lv2_host()
        .observability()
        .dispatch_return_pairs
        .iter()
        .map(|((arm, code), hits)| format!("{arm}:0x{code:08x}={hits}"))
        .collect();
    if !pairs.is_empty() {
        eprintln!("BENCH_DISPATCH_RETURN_PAIRS: {}", pairs.join(" "));
    }
    // Wait-family parks by (arm, timeout usec). timeout=0 is
    // wait-forever; nonzero registers a wake-at-guest-tick deadline.
    let parks: Vec<String> = rt
        .lv2_host()
        .observability()
        .park_timeouts
        .iter()
        .map(|((arm, timeout), hits)| format!("{arm}:t={timeout}us={hits}"))
        .collect();
    if !parks.is_empty() {
        eprintln!("BENCH_PARK_TIMEOUT_WITNESS: {}", parks.join(" "));
    }
    // Timed waits that expired with ETIMEDOUT, by primitive.
    let expiries: Vec<String> = rt
        .lv2_host()
        .observability()
        .wait_timeout_expiries
        .iter()
        .map(|(primitive, hits)| format!("{primitive}={hits}"))
        .collect();
    if !expiries.is_empty() {
        eprintln!("BENCH_WAIT_EXPIRY_WITNESS: {}", expiries.join(" "));
    }

    // sc 484 witness: how many register-module calls arrived, how
    // many took the CoreOS manual-link branch, and how the import
    // walk resolved. A frontier run with linked=0 means the branch
    // ran but bound nothing.
    let (reg_calls, reg_manual, reg_linked, reg_unresolved) =
        rt.lv2_host().observability().prx_register_module_witness();
    eprintln!(
        "BENCH_REGISTER_MODULE_WITNESS: calls={reg_calls} manual={reg_manual} linked_slots={reg_linked} unresolved_nids={reg_unresolved}"
    );

    // Event-port IPC binding: attempts vs. those that found a queue
    // registered under the key. A gap is the connect-before-create
    // race, or a producer CellGov never runs.
    let (ipc_attempts, ipc_bound) = rt.lv2_host().observability().event_port_ipc_connects;
    let keyed_queues = rt.lv2_host().keyed_event_queue_count();
    eprintln!(
        "BENCH_EVENT_PORT_WITNESS: ipc_connect_attempts={ipc_attempts} ipc_connect_bound={ipc_bound} keyed_queues={keyed_queues}"
    );

    // Null-backend inventory: which syscalls this title issued that
    // CellGov does not implement, and how often. The key set is the
    // frontier row; the counts separate a probe from a retry loop.
    let unsupported: Vec<String> = rt
        .lv2_host()
        .observability()
        .unsupported_syscalls
        .iter()
        .map(|(number, hits)| format!("{number}={hits}"))
        .collect();
    if unsupported.is_empty() {
        eprintln!("BENCH_UNSUPPORTED_SYSCALL_WITNESS: distinct=0");
    } else {
        eprintln!(
            "BENCH_UNSUPPORTED_SYSCALL_WITNESS: distinct={} {}",
            unsupported.len(),
            unsupported.join(" "),
        );
    }

    // System-IPC namespace production witnesses, both channels. A
    // silent namespace prints all zeros; the key line is suppressed
    // rather than printed empty.
    let ipc = &rt.lv2_host().observability().system_ipc_witness;
    eprintln!(
        "BENCH_SYSTEM_IPC_WITNESS: shm_creates={} shm_attaches={} shm_maps={} shm_writes={} \
         cond_creates={} cond_waits={} cond_signals={} event_queue_creates={} \
         event_queue_references={} event_queue_enqueues={} event_port_connects={} \
         distinct_keys={}",
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
    if !ipc.keys_touched.is_empty() {
        let inventory: Vec<String> = ipc
            .keys_touched
            .iter()
            .map(|(key, events)| format!("0x{key:016x}={events}"))
            .collect();
        eprintln!("BENCH_SYSTEM_IPC_KEYS: {}", inventory.join(" "));
    }

    // Virtual-UART witnesses: the PS3AV command inventory the boot
    // sent, the ids nothing answered, and the events and bytes the
    // AV manager could not deliver.
    let obs = rt.lv2_host().observability();
    if !obs.uart_cids.is_empty() {
        let sent: Vec<String> = obs
            .uart_cids
            .iter()
            .map(|(cid, hits)| format!("0x{cid:08x}={hits}"))
            .collect();
        let unknown: Vec<String> = obs
            .uart_unknown_cids
            .iter()
            .map(|(cid, hits)| format!("0x{cid:08x}={hits}"))
            .collect();
        eprintln!(
            "BENCH_UART_WITNESS: distinct={} unknown={} events_gated={} rx_overflow_bytes={} readers_queued={}",
            sent.len(),
            unknown.len(),
            obs.uart_events_gated,
            obs.uart_rx_overflow_bytes,
            obs.uart_readers_queued,
        );
        eprintln!("BENCH_UART_CIDS: {}", sent.join(" "));
        if !unknown.is_empty() {
            eprintln!("BENCH_UART_UNKNOWN_CIDS: {}", unknown.join(" "));
        }
    }

    // PRX load-miss witnesses: firmware misses stubbed with a real
    // kernel id vs loads reported CELL_ENOENT. Non-vacuity evidence
    // for the sc 480 miss arms.
    let prx_hle_stubs = rt.lv2_host().observability().prx_load_hle_stub_count;
    let prx_not_found = rt.lv2_host().observability().prx_load_not_found_count;
    eprintln!("BENCH_PRX_LOAD_WITNESS: hle_stubs={prx_hle_stubs} not_found={prx_not_found}");
    // Paths are guest-supplied. Debug quoting delimits each path and
    // escapes '"' and '\', so '=' inside a path cannot be confused
    // with the '=' before the count -- but spaces inside the quotes
    // stay literal, so a consumer must extract the quoted run first;
    // whitespace-splitting alone misparses a path containing spaces.
    let prx_misses: Vec<String> = rt
        .lv2_host()
        .observability()
        .prx_load_misses
        .iter()
        .map(|(path, hits)| format!("{path:?}={hits}"))
        .collect();
    if !prx_misses.is_empty() {
        eprintln!("BENCH_PRX_LOAD_MISSES: {}", prx_misses.join(" "));
    }

    // Terminal parking map: where every live PPU unit stopped, its
    // scheduler status, and its per-unit atomic traffic. On a
    // MaxSteps boot this names the PCs an idle loop spins at; the
    // per-unit ldarx split separates the spinners from the parked.
    for (id, unit) in rt.registry().iter() {
        if let Some(ppu) = unit
            .as_any()
            .downcast_ref::<cellgov_ppu::PpuExecutionUnit>()
        {
            let status = unit_status_label(rt.registry().effective_status(id));
            eprintln!(
                "BENCH_FINAL_UNIT_WITNESS: unit={} pc=0x{:08x} lr=0x{:08x} status={status} ldarx={} lwarx={}",
                id.raw(),
                ppu.state().pc,
                ppu.state().lr(),
                ppu.state().ldarx_executed,
                ppu.state().lwarx_executed,
            );
        }
    }

    // After the witness block: the write is host I/O, and a reader
    // that scrapes stderr must not have to wait on a disk.
    if let Some(path) = trace_path {
        std::fs::write(path, rt.trace().bytes()).unwrap_or_else(|e| {
            crate::cli::exit::die(&format!("boot bench: writing state trace to {path}: {e}"))
        });
    }

    BenchBootResult {
        run_index: opts.run_index,
        steps,
        wall,
        budget: step_budget,
        outcome,
    }
}

/// Fixed spelling for the parking-map status field: the line is
/// whitespace-delimited, so the label must never contain spaces.
fn unit_status_label(status: Option<cellgov_exec::UnitStatus>) -> &'static str {
    match status {
        Some(cellgov_exec::UnitStatus::Runnable) => "runnable",
        Some(cellgov_exec::UnitStatus::Blocked) => "blocked",
        Some(cellgov_exec::UnitStatus::Faulted) => "faulted",
        Some(cellgov_exec::UnitStatus::Finished) => "finished",
        None => "none",
    }
}

/// Run a single bench invocation and print one `BENCH_RESULT` line.
pub fn bench_boot_one_run(
    opts: BenchOptions<'_>,
    elf_data: Vec<u8>,
    authority_id: Option<u64>,
    control_flags1: Option<u32>,
    trace_path: Option<&str>,
    progress: &dyn crate::progress::ProgressSink,
) -> BenchBootResult {
    let r = bench_boot(
        opts,
        elf_data,
        authority_id,
        control_flags1,
        trace_path,
        progress,
    );
    println!("{}", format_bench_result(&r));
    r
}

/// The `BENCH_RESULT` line, the child's only channel to the parent.
///
/// Wall time travels in nanoseconds, `Duration`'s own resolution, so
/// the parent reconstructs exactly what the clock returned and
/// recomputes `steps_per_sec` from the same inputs this line was
/// printed from; `steps_per_sec` itself is carried for readers.
/// `run_index` names the measurement a captured line came from, once
/// several runs share one log.
fn format_bench_result(r: &BenchBootResult) -> String {
    format!(
        "BENCH_RESULT run_index={} steps={} wall_ns={} steps_per_sec={:.0} budget={} outcome={}",
        r.run_index,
        r.steps,
        r.wall.as_nanos(),
        r.steps_per_sec(),
        r.budget.raw(),
        r.outcome,
    )
}

/// Subprocess invocation failure surfaced by [`spawn_one_run`].
#[derive(Debug, thiserror::Error)]
pub enum SpawnError {
    #[error("subprocess spawn failed: {0}")]
    Io(#[source] std::io::Error),
    #[error("subprocess exited nonzero (status={status:?})")]
    SubprocessNonzero {
        status: Option<i32>,
        stdout: String,
        stderr: String,
    },
    #[error("BENCH_RESULT parse failed: {error}")]
    ParseFailed {
        #[source]
        error: ParseBenchError,
        stdout: String,
        stderr: String,
    },
}

impl SpawnError {
    pub fn captured_stdout(&self) -> &str {
        match self {
            Self::Io(_) => "",
            Self::SubprocessNonzero { stdout, .. } | Self::ParseFailed { stdout, .. } => stdout,
        }
    }

    pub fn captured_stderr(&self) -> &str {
        match self {
            Self::Io(_) => "",
            Self::SubprocessNonzero { stderr, .. } | Self::ParseFailed { stderr, .. } => stderr,
        }
    }
}

/// Spawn the current binary as `boot bench-once` and parse its
/// `BENCH_RESULT` line. Subprocess stderr is forwarded so warnings
/// reach the parent on the success path.
///
/// Each measurement runs in its own process. Back-to-back runs inside
/// one process drift ~60 percent in wall time on Windows, from 1 GB
/// guest-memory page-commit reuse.
///
/// Returns the parsed result alongside the subprocess stderr, which
/// carries the `BENCH_*` witness lines the anchor check reads.
fn spawn_one_run(opts: BenchOptions<'_>) -> Result<(BenchBootResult, String), SpawnError> {
    let exe = std::env::current_exe().map_err(SpawnError::Io)?;
    let mut cmd = std::process::Command::new(&exe);
    opts.encode_to_command(&mut cmd);
    let output = cmd.output().map_err(SpawnError::Io)?;
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if !output.status.success() {
        return Err(SpawnError::SubprocessNonzero {
            status: output.status.code(),
            stdout,
            stderr,
        });
    }
    if !stderr.is_empty() {
        eprint!("{stderr}");
    }
    match parse_bench_result(&stdout) {
        Ok(r) => {
            // The parent asked for this index on the command line. A
            // child that reports another index means `--run-index` no
            // longer reaches it. Every line would then read 0, and the
            // per-run attribution would be silently false.
            debug_assert_eq!(
                r.run_index, opts.run_index,
                "the child reported an index the parent did not ask for"
            );
            Ok((r, stderr))
        }
        Err(error) => Err(SpawnError::ParseFailed {
            error,
            stdout,
            stderr,
        }),
    }
}

/// Run [`bench_boot_one_run`] `policy.runs` times in separate
/// subprocesses, gate on what the runs must reproduce, and report
/// throughput.
///
/// # Panics
///
/// Panics if `policy.runs` is zero. A set of no runs has nothing to
/// compare, and the argument parser refuses the value.
pub fn bench_boot_runs(
    opts: BenchOptions<'_>,
    policy: ThroughputPolicy,
    progress: &dyn crate::progress::ProgressSink,
) -> Result<BenchRunsOutcome, SpawnError> {
    assert!(
        policy.runs > 0,
        "invariant: a run set takes at least one measurement"
    );
    // Optional trailing tokens, each carrying its own leading space
    // so the banner has no gap when both are absent.
    let mut overrides = String::new();
    if let Some(cp) = opts.checkpoint_override {
        overrides.push_str(&format!(" checkpoint={}", cp.as_cli_str()));
    }
    if let Some(b) = opts.budget_override {
        overrides.push_str(&format!(" budget={b}"));
    }
    if !opts.guest_args.is_empty() {
        // Debug quoting: guest argv entries may contain spaces.
        overrides.push_str(&format!(" guest_args={:?}", opts.guest_args));
    }
    println!(
        "boot bench: title={} elf={} max_steps={} runs={}{overrides}",
        opts.title.name(),
        opts.elf_path,
        opts.max_steps,
        policy.runs,
    );
    progress.phase(crate::progress::BenchPairPhase::Measuring.code());
    // Both counters track the same set of runs.
    progress.totals(policy.runs, policy.runs as u64);
    let mut runs: Vec<BenchBootResult> = Vec::with_capacity(policy.runs);
    let mut streams: Vec<String> = Vec::with_capacity(policy.runs);
    for index in 0..policy.runs {
        progress.item_started(&format!("run {} of {}", index + 1, policy.runs));
        let mut this_run = opts;
        this_run.run_index = index;
        let (result, stderr) = spawn_one_run(this_run)?;
        progress.advanced(1);
        progress.item_finished();
        println!(
            "  run {}: steps={} wall_ms={:.3} steps_per_sec={:.0} outcome={}",
            index + 1,
            result.steps,
            result.wall.as_secs_f64() * 1e3,
            result.steps_per_sec(),
            result.outcome,
        );
        runs.push(result);
        streams.push(stderr);
    }
    progress.phase(crate::progress::BenchPairPhase::Comparing.code());

    let determinism_failures = determinism_disagreements(&runs, &streams);
    // A set of one run has no second run to compare against, so the
    // hard gate covers nothing.
    if policy.runs == 1 {
        println!("  determinism: NOT CHECKED -- a set of one run reproduces nothing");
    }
    // Only run 1's stream reaches the anchor;
    // `determinism_disagreements` above already checked that every run
    // produced the same stream.
    let first = runs[0];
    let anchor = if !opts.check_anchor {
        AnchorVerdict::Skipped
    } else {
        let reasons = incomparable_reasons(&opts);
        match (reasons.is_empty(), opts.plan.cell) {
            // `incomparable_reasons` names a missing cell as one of its
            // reasons, so an empty list implies a cell.
            (true, Some(cell)) => check_anchor(
                &opts.title.content_id,
                cell,
                &MeasuredRun {
                    checkpoint: opts.checkpoint_override.unwrap_or(opts.plan.checkpoint),
                    steps: first.steps as u64,
                    budget: first.budget,
                    outcome: first.outcome.to_string(),
                    stderr: &streams[0],
                },
            ),
            (true, None) => unreachable!("an unnameable cell is itself an incomparable reason"),
            (false, _) => AnchorVerdict::NotComparable(reasons),
        }
    };
    match &anchor {
        AnchorVerdict::Skipped => {}
        AnchorVerdict::NotComparable(reasons) => {
            println!(
                "  anchor: NOT COMPARED against {} -- {}",
                opts.title.content_id,
                reasons.join("; ")
            );
        }
        AnchorVerdict::NotRecorded(cell) => println!(
            "  anchor: NOT RECORDED for {cell} (gates nothing) -- record it with \
             `dev record-anchors --title {}`",
            opts.title.name()
        ),
        AnchorVerdict::Match => println!(
            "  anchor: matches {} {}",
            opts.title.content_id,
            opts.plan.cell.map_or_else(String::new, CellKey::label)
        ),
        AnchorVerdict::Drift(f) => {
            println!(
                "  anchor: {} disagreement(s) vs {} {}",
                f.len(),
                opts.title.content_id,
                opts.plan.cell.map_or_else(String::new, CellKey::label)
            )
        }
    }

    let throughput = throughput_verdict(&runs);
    print_throughput(throughput, policy);
    let gate = classify_runs(&determinism_failures, &anchor, throughput, policy);
    progress.finished();
    if gate == BenchGate::DeterminismBreak {
        println!("  determinism: BREAK");
        for failure in &determinism_failures {
            println!("    {failure}");
        }
        for line in locate_divergence(opts, &runs) {
            println!("    {line}");
        }
    }
    Ok(BenchRunsOutcome {
        runs,
        throughput,
        gate,
        anchor_failures: match anchor {
            AnchorVerdict::Drift(f) => f,
            _ => Vec::new(),
        },
        determinism_failures,
    })
}

fn print_throughput(verdict: ThroughputVerdict, policy: ThroughputPolicy) {
    println!("{}", throughput_line(verdict, policy));
}

/// A set of one run spreads against nothing, and a range over one
/// measurement is 0%. That would read as an agreement, so the
/// single-run line names the estimate and says the spread went
/// unchecked.
fn throughput_line(verdict: ThroughputVerdict, policy: ThroughputPolicy) -> String {
    let outcome = if policy.strict && !verdict.is_measured() {
        "FAIL (--strict-perf)"
    } else if verdict.is_measured() {
        "OK"
    } else {
        "SKIPPED"
    };
    match verdict {
        ThroughputVerdict::Measured { min, .. } if policy.runs < 2 => format!(
            "  throughput: min_ms={:.3} over 1 run, spread NOT CHECKED -- one \
             measurement spreads against nothing => {outcome}",
            min.as_secs_f64() * 1e3,
        ),
        ThroughputVerdict::Measured { min, spread_pct } => format!(
            "  throughput: min_ms={:.3} over {} run(s), spread {spread_pct:.2}% \
             (ceiling {BENCH_SPREAD_CEILING_PCT}%) => {outcome}",
            min.as_secs_f64() * 1e3,
            policy.runs,
        ),
        ThroughputVerdict::Inconclusive { min, spread_pct } => format!(
            "  throughput: INCONCLUSIVE -- min_ms={:.3} over {} run(s), spread \
             {spread_pct:.2}% above the {BENCH_SPREAD_CEILING_PCT}% ceiling; the host was \
             busy, so no throughput claim is made => {outcome}",
            min.as_secs_f64() * 1e3,
            policy.runs,
        ),
        ThroughputVerdict::Unmeasurable => format!(
            "  throughput: INCONCLUSIVE -- a run reported a zero wall, so there is no \
             spread to compare => {outcome}"
        ),
    }
}

/// Every way the runs of a set failed to reproduce each other.
///
/// Run 1 is the reference for every comparison, so one counter that
/// moves yields one finding per run that moved.
fn determinism_disagreements(runs: &[BenchBootResult], streams: &[String]) -> Vec<String> {
    // The one production caller fills both vectors from the same loop,
    // so a shorter `streams` cannot reach this point.
    debug_assert_eq!(
        runs.len(),
        streams.len(),
        "every run of a set carries the stream it printed"
    );
    let mut out = Vec::new();
    for (index, run) in runs.iter().enumerate().skip(1) {
        if run.steps != runs[0].steps || run.outcome != runs[0].outcome {
            out.push(format!(
                "run 1 retired {} steps ending {}, run {} retired {} steps ending {}",
                runs[0].steps,
                runs[0].outcome,
                index + 1,
                run.steps,
                run.outcome,
            ));
        }
        // Each child re-resolves its own composition, so the budget is
        // a per-run result. Only run 1's budget reaches the anchor
        // check. A budget that moves between runs is otherwise
        // invisible: it retires a different trajectory under an
        // unmoved step count.
        if run.budget != runs[0].budget {
            out.push(format!(
                "run 1 ran at budget {}, run {} ran at budget {}",
                runs[0].budget,
                index + 1,
                run.budget,
            ));
        }
        out.extend(witness_disagreements(
            "run 1",
            &streams[0],
            &format!("run {}", index + 1),
            &streams[index],
        ));
    }
    out
}

/// Retired steps above which a break is localized by hand.
///
/// `DeterminismCheck` records a state hash per retired instruction, so
/// a traced boot costs orders of magnitude more time and memory than
/// the measurement it re-runs. Past this cap the report names the two
/// commands and runs neither.
const LOCALIZE_MAX_STEPS: usize = 25_000;

/// Localize a determinism break to its first divergent step.
///
/// Two more boots run under `DeterminismCheck`, and the two traces go
/// through the same comparison `diff diverge` uses. A break that
/// `DeterminismCheck` mode does not reproduce reports as identical.
///
/// The cap reads the longest run: when a break moves the step count,
/// run 1 bounds neither re-run.
fn locate_divergence(opts: BenchOptions<'_>, runs: &[BenchBootResult]) -> Vec<String> {
    let mut out = Vec::new();
    let steps = runs.iter().map(|r| r.steps).max().unwrap_or(0);
    if steps > LOCALIZE_MAX_STEPS {
        out.push(format!(
            "diverge: not run automatically -- the boot retires {steps} steps, past the \
             {LOCALIZE_MAX_STEPS} a traced re-run is affordable at. Localize it by hand:"
        ));
        for i in 0..2 {
            out.push(format!(
                "  cellgov boot bench-once --title {} --save-state-trace run{i}.state",
                opts.title.name()
            ));
        }
        out.push("  cellgov diff diverge run0.state run1.state".to_string());
        return out;
    }
    // This prints before the two boots below, which run in
    // DeterminismCheck mode with no progress bar and take far longer
    // than the measurements did.
    println!(
        "    diverge: re-running the boot twice under --save-state-trace to localize the \
         break; this is slower than the measurement was"
    );
    let pid = std::process::id();
    let paths: Vec<PathBuf> = (0..2)
        .map(|i| std::env::temp_dir().join(format!("cellgov-bench-diverge-{pid}-{i}.state")))
        .collect();
    let mut traces = Vec::with_capacity(paths.len());
    for (i, path) in paths.iter().enumerate() {
        let Some(text) = path.to_str() else {
            out.push(format!(
                "cannot localize: the temporary trace path {} is not valid UTF-8",
                path.display()
            ));
            cleanup_traces(&paths);
            return out;
        };
        let mut traced = opts;
        // The offset puts these indices outside the measured set's
        // range, so a captured log cannot read a diagnostic boot as a
        // measurement.
        traced.run_index = opts.run_index + 1000 + i;
        let exe = match std::env::current_exe() {
            Ok(e) => e,
            Err(e) => {
                out.push(format!("cannot localize: current_exe: {e}"));
                cleanup_traces(&paths);
                return out;
            }
        };
        let mut cmd = std::process::Command::new(exe);
        traced.encode_to_command(&mut cmd);
        cmd.arg("--save-state-trace").arg(text);
        match cmd.output() {
            Ok(o) if o.status.success() => {}
            Ok(o) => {
                out.push(format!(
                    "cannot localize: the traced re-run exited {:?}",
                    o.status.code()
                ));
                // The child's own stderr is the only account of why it
                // refused.
                out.extend(stderr_tail(&o.stderr));
                cleanup_traces(&paths);
                return out;
            }
            Err(e) => {
                out.push(format!("cannot localize: spawning the traced re-run: {e}"));
                cleanup_traces(&paths);
                return out;
            }
        }
        match std::fs::read(path) {
            Ok(bytes) => traces.push(bytes),
            Err(e) => {
                out.push(format!("cannot localize: reading {}: {e}", path.display()));
                cleanup_traces(&paths);
                return out;
            }
        }
    }
    cleanup_traces(&paths);
    out.push(format_diverge(&cellgov_compare::diverge(
        &traces[0], &traces[1],
    )));
    out
}

/// Lines a failing child left on stderr, indented for the report.
///
/// A boot that refuses can print a whole witness block first, and the
/// refusal is the last thing it says.
fn stderr_tail(stderr: &[u8]) -> Vec<String> {
    const TAIL_LINES: usize = 8;
    let text = String::from_utf8_lossy(stderr);
    let mut tail: Vec<String> = text
        .lines()
        .rev()
        .take(TAIL_LINES)
        .map(|l| format!("  {l}"))
        .collect();
    tail.reverse();
    tail
}

fn cleanup_traces(paths: &[PathBuf]) {
    for path in paths {
        // Best effort: this path already reports a failure, and a
        // leftover file in the OS temp directory adds nothing to it.
        drop(std::fs::remove_file(path));
    }
}

/// One line naming where two traced re-runs first disagree.
fn format_diverge(report: &cellgov_compare::DivergeReport) -> String {
    use cellgov_compare::{DivergeField, DivergeReport};
    match report {
        DivergeReport::Identical { count } => format!(
            "diverge: the two traced re-runs matched over {count} PpuStateHash record(s); \
             the break did not reproduce under DeterminismCheck mode"
        ),
        DivergeReport::Differs {
            step,
            a_pc,
            b_pc,
            a_hash,
            b_hash,
            field,
        } => {
            let field = match field {
                DivergeField::Pc => "pc",
                DivergeField::Hash => "hash",
            };
            format!(
                "diverge: first divergent step={step} field={field} \
                 a_pc=0x{a_pc:x} b_pc=0x{b_pc:x} a_hash=0x{a_hash:x} b_hash=0x{b_hash:x}"
            )
        }
        DivergeReport::LengthDiffers {
            common_count,
            a_count,
            b_count,
        } => format!(
            "diverge: the traced re-runs agreed over {common_count} record(s) then ran to \
             different lengths (a={a_count}, b={b_count})"
        ),
        // Both sides render, as `cellgov diff diverge` renders them.
        DivergeReport::CorruptTrace {
            common_count,
            a_error,
            b_error,
        } => {
            let a = a_error
                .as_ref()
                .map_or_else(|| "ok".to_string(), ToString::to_string);
            let b = b_error
                .as_ref()
                .map_or_else(|| "ok".to_string(), ToString::to_string);
            format!(
                "diverge: a traced re-run failed to decode after {common_count} record(s), so \
                 nothing past the cut was compared (a: {a}, b: {b})"
            )
        }
    }
}

/// How a run compared against its cell's committed anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AnchorVerdict {
    /// The check was not requested.
    Skipped,
    /// The comparison is meaningless for this invocation; each string
    /// names one cause.
    NotComparable(Vec<String>),
    /// No anchor is committed for the cell this run composed, so there
    /// is nothing to compare against.
    NotRecorded(String),
    /// The run reproduced every recorded value.
    Match,
    /// Every disagreement found, in the order the comparison made them.
    Drift(Vec<String>),
}

/// Why this invocation cannot be held against the cell's committed
/// anchor.
///
/// The anchor is measured by `dev record-anchors`, which boots the cell
/// under what its manifest row declares and nothing else. An override
/// that moves the trajectory yields a legitimately different run, so
/// gating it would report a regression that is not one. `--prescan` is
/// absent from the list because it only prints a decode report before
/// execution.
fn incomparable_reasons(opts: &BenchOptions<'_>) -> Vec<String> {
    let mut reasons = Vec::new();
    if let Some(dir) = opts.selection.firmware_dir {
        reasons.push(format!(
            "--firmware-dir {dir} is unmanaged: the run carries no firmware version, so nothing \
             names the cell an anchor would be filed under"
        ));
    } else if opts.plan.cell.is_none() {
        reasons.push(format!(
            "{} composed no cell: an anchor is keyed by (content id, firmware, game version), \
             and this run named no firmware version or no game version to key on",
            opts.title.name()
        ));
    }
    if opts.max_steps as u64 != opts.plan.max_steps {
        reasons.push(format!(
            "--max-steps {} differs from the {} the cell's anchor is recorded under",
            opts.max_steps, opts.plan.max_steps
        ));
    }
    if let Some(cp) = opts.checkpoint_override {
        if cp != opts.plan.checkpoint {
            reasons.push(format!(
                "--checkpoint {} overrides the cell's checkpoint {}",
                cp.as_cli_str(),
                opts.plan.checkpoint.as_cli_str()
            ));
        }
    }
    if let Some(b) = opts.budget_override {
        reasons.push(format!("--budget {b} overrides the manifest budget"));
    }
    if opts.strict_reserved {
        reasons.push("--strict-reserved changes reserved-region write handling".to_string());
    }
    if !opts.guest_args.is_empty() {
        reasons.push(format!(
            "--guest-arg supplies {} guest argv entries; the anchor is recorded with none",
            opts.guest_args.len()
        ));
    }
    reasons
}

/// Witness-level disagreements between two runs of a set.
///
/// The steps/outcome comparison cannot see a counter that moved
/// without changing either, and the anchor check reads one run's
/// stream; without this, a witness that is nondeterministic across
/// runs passes the gate whenever it happens to match the anchor.
fn witness_disagreements(
    a_label: &str,
    a_stderr: &str,
    b_label: &str,
    b_stderr: &str,
) -> Vec<String> {
    let mut out = Vec::new();
    let mut parsed = Vec::new();
    for (label, stderr) in [(a_label, a_stderr), (b_label, b_stderr)] {
        match parse_witness_lines(stderr) {
            Ok(w) => parsed.push(w),
            Err(errs) => out.extend(
                errs.iter()
                    .map(|e| format!("{label} witness line did not parse: {e}")),
            ),
        }
    }
    let [a, b] = parsed.as_slice() else {
        return out;
    };
    for line in a.seen_lines.symmetric_difference(&b.seen_lines) {
        let present = if a.seen_lines.contains(line) {
            a_label
        } else {
            b_label
        };
        out.push(format!("witness line {line} appeared in {present} only"));
    }
    for (name, x) in &a.values {
        let Some(y) = b.values.get(name) else {
            out.push(format!(
                "witness {name}: {a_label} {x}, absent from {b_label}"
            ));
            continue;
        };
        if x != y {
            out.push(format!("witness {name}: {a_label} {x} != {b_label} {y}"));
        }
    }
    for name in b.values.keys() {
        if !a.values.contains_key(name) {
            out.push(format!(
                "witness {name}: {b_label} {}, absent from {a_label}",
                b.values[name]
            ));
        }
    }
    out
}

/// How the triple a summary embeds disagrees with the one the run
/// composed.
///
/// The anchor's directory names the cell it is filed under; the
/// embedded triple is what the recording run itself composed. A file
/// whose two accounts disagree was measured elsewhere, so the numbers
/// below compare two different cells.
fn mislabelled_anchor(recorded: &RunIdentity, run: &RunIdentity) -> Vec<String> {
    let mut failures = Vec::new();
    if recorded.firmware != run.firmware {
        failures.push(format!(
            "the anchor was measured against a different firmware: recorded {}, ran {}",
            render_firmware(recorded.firmware.as_ref()),
            render_firmware(run.firmware.as_ref()),
        ));
    }
    if recorded.game != run.game {
        failures.push(format!(
            "the anchor was measured against a different title version: recorded {}, ran {}",
            render_game(recorded.game.as_ref()),
            render_game(run.game.as_ref()),
        ));
    }
    failures
}

/// How a report names the firmware half.
///
/// The comparison covers every field, so the report renders every
/// field. Two entries installed from different PUPs can carry one
/// version string. The version alone would then print the same value
/// on both sides of a disagreement.
fn render_firmware(half: Option<&FirmwareIdentity>) -> String {
    half.map_or_else(unidentified, |f| {
        format!(
            "{} (image {}, pup sha256 {})",
            f.version, f.image_version, f.pup_sha256
        )
    })
}

/// How a report names the game half. Renders every compared field for
/// the reason [`render_firmware`] gives.
fn render_game(half: Option<&GameIdentity>) -> String {
    half.map_or_else(unidentified, |g| {
        format!("{} {} (app_ver {})", g.title_id, g.version, g.app_ver)
    })
}

/// The half a run had nothing to name.
fn unidentified() -> String {
    "(unidentified)".to_string()
}

/// Compare a run against a loaded anchor, returning every
/// disagreement.
///
/// Mirrors the comparison in `tests/title_witnesses.rs`: the two must
/// agree, or `boot bench` would pass a run the witness suite rejects.
fn anchor_disagreements(
    baseline: &BootSummary,
    identity: &RunIdentity,
    checkpoint: manifest::CheckpointTrigger,
    steps: u64,
    budget: Budget,
    outcome: &str,
    observed: &ParsedWitnesses,
) -> Vec<String> {
    let mut failures = mislabelled_anchor(&baseline.identity, identity);
    // The checkpoint is a manifest row an edit can move after the
    // anchor was recorded. A stop condition the run never reaches
    // leaves the steps, the outcome and the witnesses intact, so
    // nothing else in this comparison sees the move.
    let recorded_checkpoint = crate::paths::checkpoint_kind(checkpoint);
    if recorded_checkpoint != baseline.checkpoint {
        failures.push(format!(
            "checkpoint {} != recorded {}",
            recorded_checkpoint.as_markdown_label(),
            baseline.checkpoint.as_markdown_label()
        ));
    }
    if steps != baseline.steps {
        failures.push(format!("steps {steps} != recorded {}", baseline.steps));
    }
    // `steps * budget` is the anchor's instruction count, so a moved
    // budget retires a different trajectory under an unmoved step count.
    if budget != baseline.budget {
        failures.push(format!("budget {budget} != recorded {}", baseline.budget));
    }
    // Display, not Debug: `BootOutcome`'s `FromStr` round-trips the
    // Display form, and the two disagree for `PcReached`, whose Debug
    // prints the address in decimal.
    let recorded_outcome = baseline.outcome.to_string();
    if outcome != recorded_outcome {
        failures.push(format!("outcome {outcome} != recorded {recorded_outcome}"));
    }
    if baseline.witnesses.is_empty() {
        failures.push("anchor records no witnesses".to_string());
        return failures;
    }
    for failure in check_all(&baseline.witnesses, observed) {
        failures.push(failure.to_string());
    }
    for name in unrecorded(&baseline.witnesses, &observed.values) {
        failures.push(format!(
            "witness {name} is emitted but not recorded in the anchor"
        ));
    }
    failures
}

/// What one measured run of the set produced, in the terms the anchor
/// records.
struct MeasuredRun<'a> {
    /// Stop condition the run was taken at.
    checkpoint: manifest::CheckpointTrigger,
    steps: u64,
    budget: Budget,
    outcome: String,
    /// The measuring process's own stderr: the witness lines, and the
    /// `RUN_IDENTITY` line naming what it composed.
    stderr: &'a str,
}

/// Load the anchor for one cell of `content_id` and compare `run`
/// against it.
///
/// An unreadable or unparseable anchor is a disagreement, not a skip:
/// only a genuinely absent file means "nothing recorded yet".
///
/// [`workspace_root`] is compiled in, so a binary invoked outside the
/// tree it was built from reaches no anchor at all. That says nothing
/// about what is recorded, so it reports as
/// [`AnchorVerdict::NotComparable`] rather than letting every cell
/// look unrecorded.
fn check_anchor(content_id: &str, cell: &CellKey, run: &MeasuredRun<'_>) -> AnchorVerdict {
    check_anchor_under(&workspace_root(), content_id, cell, run)
}

fn check_anchor_under(
    root: &Path,
    content_id: &str,
    cell: &CellKey,
    run: &MeasuredRun<'_>,
) -> AnchorVerdict {
    if !root.is_dir() {
        return AnchorVerdict::NotComparable(vec![format!(
            "the compiled-in workspace root {} is not present on this machine, so no \
             committed anchor is reachable",
            root.display()
        )]);
    }
    let path = boot_anchor_path(root, content_id, cell);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return AnchorVerdict::NotRecorded(cell.label())
        }
        Err(e) => {
            return AnchorVerdict::Drift(vec![format!("read {}: {e}", path.display())]);
        }
    };
    let baseline: BootSummary = match serde_json::from_str(&text) {
        Ok(b) => b,
        Err(e) => {
            return AnchorVerdict::Drift(vec![format!("parse {}: {e}", path.display())]);
        }
    };
    let observed = match parse_witness_lines(run.stderr) {
        Ok(w) => w,
        Err(errs) => {
            return AnchorVerdict::Drift(
                errs.iter()
                    .map(|e| format!("malformed witness line: {e}"))
                    .collect(),
            );
        }
    };
    // The triple comes out of the measuring child's own stream. The
    // steps and the witnesses below came out of that same stream, and
    // a triple the parent resolved would hide the mismatch this
    // comparison exists to catch.
    let identity = match RunIdentity::parse_sentinel_lines(run.stderr) {
        Ok(Some(i)) => i,
        Ok(None) => {
            return AnchorVerdict::Drift(vec![format!(
                "the measured run printed no {RUN_IDENTITY_SENTINEL} line, so nothing says \
                 which firmware and title version produced these numbers"
            )])
        }
        Err(e) => return AnchorVerdict::Drift(vec![e.to_string()]),
    };
    let failures = anchor_disagreements(
        &baseline,
        &identity,
        run.checkpoint,
        run.steps,
        run.budget,
        &run.outcome,
        &observed,
    );
    if failures.is_empty() {
        AnchorVerdict::Match
    } else {
        AnchorVerdict::Drift(failures)
    }
}

/// Order matters. A determinism break makes the witness stream
/// meaningless. An anchor disagreement outranks the throughput
/// verdict, so a contended host cannot mask a real regression behind a
/// timing failure.
///
/// Throughput reaches the gate only under `policy.strict`: a busy host
/// inflates the spread of a run that regressed nothing.
fn classify_runs(
    determinism_failures: &[String],
    anchor: &AnchorVerdict,
    throughput: ThroughputVerdict,
    policy: ThroughputPolicy,
) -> BenchGate {
    if !determinism_failures.is_empty() {
        return BenchGate::DeterminismBreak;
    }
    if matches!(anchor, AnchorVerdict::Drift(_)) {
        return BenchGate::AnchorDrift;
    }
    if policy.strict && !throughput.is_measured() {
        return BenchGate::SpreadExceeded;
    }
    BenchGate::Pass
}

fn throughput_verdict(runs: &[BenchBootResult]) -> ThroughputVerdict {
    if runs.is_empty() {
        return ThroughputVerdict::Unmeasurable;
    }
    let mut min = Duration::MAX;
    let mut max = Duration::ZERO;
    for run in runs {
        if run.wall.is_zero() {
            return ThroughputVerdict::Unmeasurable;
        }
        min = min.min(run.wall);
        max = max.max(run.wall);
    }
    let spread_pct = 100.0 * (max.as_secs_f64() - min.as_secs_f64()) / min.as_secs_f64();
    if spread_pct > BENCH_SPREAD_CEILING_PCT {
        ThroughputVerdict::Inconclusive { min, spread_pct }
    } else {
        ThroughputVerdict::Measured { min, spread_pct }
    }
}

/// Failure mode while parsing a `BENCH_RESULT` line out of subprocess
/// stdout.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseBenchError {
    #[error("no BENCH_RESULT line")]
    NoResultLine,
    #[error("more than one BENCH_RESULT line")]
    DuplicateResultLine,
    #[error("BENCH_RESULT: missing run_index= field")]
    MissingRunIndex,
    #[error("BENCH_RESULT: malformed run_index={0:?}")]
    MalformedRunIndex(String),
    #[error("BENCH_RESULT: missing steps= field")]
    MissingSteps,
    #[error("BENCH_RESULT: malformed steps={0:?}")]
    MalformedSteps(String),
    #[error("BENCH_RESULT: missing wall_ns= field")]
    MissingWallNs,
    #[error("BENCH_RESULT: malformed wall_ns={0:?}")]
    MalformedWallNs(String),
    #[error("BENCH_RESULT: missing budget= field")]
    MissingBudget,
    #[error("BENCH_RESULT: malformed budget={0:?}")]
    MalformedBudget(String),
    #[error("BENCH_RESULT: missing outcome= field")]
    MissingOutcome,
    #[error("BENCH_RESULT: malformed outcome={token:?}: {source}")]
    UnparseableOutcome {
        token: String,
        #[source]
        source: BootOutcomeParseError,
    },
}

/// Parse the `BENCH_RESULT run_index=I steps=N wall_ns=M
/// steps_per_sec=X budget=B outcome=O` line out of captured stdout.
///
/// `wall_ns` must fit a `u64` (about 584 years); the child's `u128`
/// print never exceeds that for a real run, and a larger value is
/// reported as malformed rather than clamped.
pub(crate) fn parse_bench_result(stdout: &str) -> Result<BenchBootResult, ParseBenchError> {
    let mut iter = stdout.lines().filter(|l| l.starts_with("BENCH_RESULT "));
    let line = iter.next().ok_or(ParseBenchError::NoResultLine)?;
    if iter.next().is_some() {
        return Err(ParseBenchError::DuplicateResultLine);
    }
    let mut run_index: Option<usize> = None;
    let mut steps: Option<usize> = None;
    let mut wall_ns: Option<u64> = None;
    let mut budget: Option<u64> = None;
    let mut outcome_token: Option<String> = None;
    let mut reported_sps: Option<f64> = None;
    for tok in line.split_whitespace().skip(1) {
        if let Some(v) = tok.strip_prefix("run_index=") {
            run_index = Some(
                v.parse()
                    .map_err(|_| ParseBenchError::MalformedRunIndex(v.to_string()))?,
            );
        } else if let Some(v) = tok.strip_prefix("steps=") {
            steps = Some(
                v.parse()
                    .map_err(|_| ParseBenchError::MalformedSteps(v.to_string()))?,
            );
        } else if let Some(v) = tok.strip_prefix("wall_ns=") {
            wall_ns = Some(
                v.parse()
                    .map_err(|_| ParseBenchError::MalformedWallNs(v.to_string()))?,
            );
        } else if let Some(v) = tok.strip_prefix("budget=") {
            budget = Some(
                v.parse()
                    .map_err(|_| ParseBenchError::MalformedBudget(v.to_string()))?,
            );
        } else if let Some(v) = tok.strip_prefix("steps_per_sec=") {
            reported_sps = v.parse().ok();
        } else if let Some(v) = tok.strip_prefix("outcome=") {
            outcome_token = Some(v.to_string());
        } else {
            eprintln!(
                "parse_bench_result: warning: unknown token {tok:?} in BENCH_RESULT line; parser may be stale"
            );
        }
    }
    let run_index = run_index.ok_or(ParseBenchError::MissingRunIndex)?;
    let steps = steps.ok_or(ParseBenchError::MissingSteps)?;
    let wall_ns = wall_ns.ok_or(ParseBenchError::MissingWallNs)?;
    let budget = budget.ok_or(ParseBenchError::MissingBudget)?;
    let outcome_token = outcome_token.ok_or(ParseBenchError::MissingOutcome)?;
    let outcome = BootOutcome::from_str(&outcome_token).map_err(|source| {
        ParseBenchError::UnparseableOutcome {
            token: outcome_token.clone(),
            source,
        }
    })?;
    let wall = Duration::from_nanos(wall_ns);
    let result = BenchBootResult {
        run_index,
        steps,
        wall,
        budget: Budget::new(budget),
        outcome,
    };
    // The transport is lossless, so the parent recomputes
    // steps_per_sec from exactly the child's inputs and the only
    // admissible difference is the child's `{:.0}` print rounding.
    // Anything wider means the writer and this reader disagree on
    // the line's units.
    if let Some(reported) = reported_sps {
        let computed = result.steps_per_sec();
        let tolerance = 0.5 + computed.abs() * f64::EPSILON;
        debug_assert!(
            (reported - computed).abs() <= tolerance,
            "BENCH_RESULT steps_per_sec drift: reported={reported} computed={computed} (tolerance={tolerance})"
        );
    }
    Ok(result)
}

#[cfg(test)]
#[path = "tests/bench_tests.rs"]
mod tests;

/// The encoded child invocation, read back through the command tree the
/// child parses it with.
#[cfg(test)]
mod child_command_tests {
    use clap::Parser as _;

    use super::*;
    use crate::cli::parse::{BootCommand, Cli, Command};

    fn bench_manifest() -> TitleManifest {
        use manifest::{CheckpointTrigger, Distribution, GameSource};
        TitleManifest {
            content_id: "CG_TEST".to_string(),
            short_name: "test".to_string(),
            display_name: "test".to_string(),
            eboot_candidates: vec!["EBOOT.BIN".to_string()],
            year: 2007,
            developer: "test-developer".to_string(),
            engine: "test-engine".to_string(),
            distribution: Distribution::PsnHdd,
            rap_filename: None,
            bench_max_steps: Some(4_000),
            checkpoint: CheckpointTrigger::ProcessExit,
            source: GameSource::Hdd,
            rsx_mirror: false,
            rsx_consume: false,
            content: None,
            mounts: Vec::new(),
            matrix: Vec::new(),
        }
    }

    /// Every forwarded flag must survive the round trip. A run set
    /// re-enters the binary as a child process, and it forwards the
    /// selection flags for the child to resolve on its own. A spelling
    /// the child parses differently makes the runs measure different
    /// things while the gate still reports agreement. See
    /// `docs/architecture/title_harness.md`, "Title anchors and
    /// witnesses".
    #[test]
    fn the_encoded_child_invocation_parses_back_into_the_same_run() {
        let title = bench_manifest();
        let identity = cellgov_compare::RunIdentity::default();
        let guest_args = vec!["--trace".to_string(), "argv1".to_string()];
        let cell = CellKey {
            fw: "4.91".to_string(),
            game_ver: Some("02.51".to_string()),
        };
        let opts = BenchOptions {
            title: &title,
            elf_path: "EBOOT.BIN",
            max_steps: 4_000,
            plan: AnchorPlan {
                cell: Some(&cell),
                max_steps: 4_000,
                checkpoint: manifest::CheckpointTrigger::ProcessExit,
            },
            // The resolved directory, which must not reach the child.
            firmware_dir: Some("resolved/4.91/dev_flash/sys/external"),
            composed_mounts: &[],
            identity: &identity,
            selection: SelectionArgs {
                fw: Some("4.91"),
                game_ver: Some("02.51"),
                firmware_dir: None,
                vfs_root: Some("elsewhere/dev_hdd0"),
            },
            strict_reserved: true,
            checkpoint_override: Some(manifest::CheckpointTrigger::Pc(0x1_0000)),
            budget_override: Some(Budget::new(512)),
            prescan: true,
            guest_args: &guest_args,
            check_anchor: true,
            run_index: 0,
        };

        let mut cmd = std::process::Command::new("cellgov");
        opts.encode_to_command(&mut cmd);
        let mut argv = vec!["cellgov".to_string()];
        argv.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));

        let cli = Cli::try_parse_from(&argv)
            .unwrap_or_else(|e| panic!("child argv {argv:?} does not parse: {e}"));
        assert_eq!(
            cli.globals.vfs_root.as_deref(),
            Some(Path::new("elsewhere/dev_hdd0")),
        );
        // `bench-once`, never `bench`: the gating set must not spawn
        // another gating set.
        let Command::Boot(BootCommand::BenchOnce(child)) = cli.command else {
            panic!("child argv {argv:?} did not select `boot bench-once`");
        };
        assert_eq!(child.run_index, Some(0));
        assert_eq!(child.selector.title.as_deref(), Some(title.name()));
        assert_eq!(child.selector.content_id, None);
        assert_eq!(child.selector.title_manifest, None);
        assert_eq!(child.selection.fw.as_deref(), Some("4.91"));
        assert_eq!(child.selection.game_ver.as_deref(), Some("02.51"));
        assert_eq!(child.selection.firmware_dir, None);
        assert_eq!(child.max_steps, Some(4_000));
        assert_eq!(child.budget, Some(512));
        assert_eq!(
            child.checkpoint,
            Some(manifest::CheckpointTrigger::Pc(0x1_0000)),
        );
        assert!(child.prescan);
        assert!(child.strict_reserved);
        assert_eq!(child.guest_arg, guest_args);
        // A child bar renders into the pipe the parent parses.
        assert!(cli.globals.no_progress);
    }

    #[test]
    fn an_unmanaged_firmware_tree_reaches_the_child_as_the_flag() {
        let title = bench_manifest();
        let identity = cellgov_compare::RunIdentity::default();
        let opts = BenchOptions {
            title: &title,
            elf_path: "EBOOT.BIN",
            max_steps: 4_000,
            plan: AnchorPlan {
                cell: None,
                max_steps: 4_000,
                checkpoint: manifest::CheckpointTrigger::ProcessExit,
            },
            firmware_dir: Some("elsewhere/sys/external"),
            composed_mounts: &[],
            identity: &identity,
            selection: SelectionArgs {
                firmware_dir: Some("elsewhere/sys/external"),
                ..SelectionArgs::default()
            },
            strict_reserved: false,
            checkpoint_override: None,
            budget_override: None,
            prescan: false,
            guest_args: &[],
            check_anchor: true,
            run_index: 0,
        };

        let mut cmd = std::process::Command::new("cellgov");
        opts.encode_to_command(&mut cmd);
        let mut argv = vec!["cellgov".to_string()];
        argv.extend(cmd.get_args().map(|a| a.to_string_lossy().into_owned()));

        let cli = Cli::try_parse_from(&argv)
            .unwrap_or_else(|e| panic!("child argv {argv:?} does not parse: {e}"));
        let Command::Boot(BootCommand::BenchOnce(child)) = cli.command else {
            panic!("child argv {argv:?} did not select `boot bench-once`");
        };
        assert_eq!(
            child.selection.firmware_dir.as_deref(),
            Some(Path::new("elsewhere/sys/external")),
        );
        assert_eq!(child.selection.fw, None);
    }
}
