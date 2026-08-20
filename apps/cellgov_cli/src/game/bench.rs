//! `bench-boot` / `bench-boot-once` / `bench-boot-pair` machinery.
//! A pair runs two subprocesses and gates on per-run agreement.

use std::path::Path;
use std::str::FromStr;
use std::time::Instant;

use cellgov_compare::witness_parse::{parse_witness_lines, ParsedWitnesses};
use cellgov_compare::witnesses::{check_all, unrecorded};
use cellgov_compare::{BootOutcome, BootOutcomeParseError, BootSummary};
use cellgov_time::Budget;

use super::boot;
use super::manifest::{self, TitleManifest};
use super::step_loop::bench_step_loop;
use crate::paths::{baseline_path, workspace_root, DEFAULT_BENCH_MAX_STEPS};

/// Wall-time disagreement that trips the pair gate, as a percentage
/// of the faster run.
pub const BENCH_AGREEMENT_GATE_PCT: f64 = 5.0;

/// Inputs common to every bench-boot entry point.
#[derive(Debug, Clone, Copy)]
pub struct BenchOptions<'a> {
    pub title: &'a TitleManifest,
    pub elf_path: &'a str,
    pub max_steps: usize,
    pub firmware_dir: Option<&'a str>,
    pub strict_reserved: bool,
    pub checkpoint_override: Option<manifest::CheckpointTrigger>,
    pub budget_override: Option<Budget>,
    /// When true, scan the title ELF for unimplemented PPU
    /// encodings before execution and print the gap report.
    pub prescan: bool,
    /// Guest argv for the primary thread, `argv[0]` included. Empty
    /// keeps the no-args entry state (r3..r6 = 0).
    pub guest_args: &'a [String],
    /// Compare the run against the title's committed anchor. Cleared
    /// by `--no-anchor-check` for the re-record workflow, where the
    /// anchor is expected to disagree.
    pub check_anchor: bool,
}

impl BenchOptions<'_> {
    /// Append the `bench-boot-once` CLI form of this struct onto `cmd`.
    fn encode_to_command(&self, cmd: &mut std::process::Command) {
        cmd.arg("bench-boot-once")
            .arg("--title")
            .arg(self.title.name())
            .arg("--max-steps")
            .arg(self.max_steps.to_string());
        if let Some(d) = self.firmware_dir {
            cmd.arg("--firmware-dir").arg(d);
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

/// One completed bench run: retired step count, wall duration, and
/// terminal [`BootOutcome`].
#[derive(Debug, Clone, Copy)]
pub struct BenchBootResult {
    pub steps: usize,
    pub wall: std::time::Duration,
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

/// Gate verdict for [`bench_boot_pair`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchGate {
    /// Both runs agree on steps + outcome; wall drift within
    /// [`BENCH_AGREEMENT_GATE_PCT`].
    Pass,
    /// Runs disagreed on retired step count or boot outcome.
    DeterminismBreak,
    /// The run disagreed with the title's committed anchor.
    AnchorDrift,
    /// Wall drift exceeded the gate.
    WallDriftExceeded,
    /// Wall measurement was zero / non-finite.
    WallUnmeasurable,
}

/// Result of one [`bench_boot_pair`] invocation.
#[derive(Debug, Clone)]
pub struct BenchPairOutcome {
    pub run1: BenchBootResult,
    pub run2: BenchBootResult,
    pub drift_pct: Option<f64>,
    pub gate: BenchGate,
    /// Every anchor disagreement found, empty unless `gate` is
    /// [`BenchGate::AnchorDrift`].
    pub anchor_failures: Vec<String>,
}

/// Run one boot with the minimum step-loop bookkeeping needed to
/// detect termination.
///
/// RSX-init coupling: `set_rsx_mirror_writes` is driven by the
/// manifest's declared checkpoint, not the runtime-overridable
/// `checkpoint`, so a `--checkpoint pc=ADDR` override does not change
/// the boot trajectory's init path.
pub fn bench_boot(
    opts: BenchOptions<'_>,
    elf_data: Vec<u8>,
    authority_id: Option<u64>,
    control_flags1: Option<u32>,
) -> BenchBootResult {
    let prepared = boot::prepare(boot::PrepareOptions {
        title: opts.title,
        elf_path: opts.elf_path,
        elf_data,
        authority_id,
        control_flags1,
        firmware_dir: opts.firmware_dir,
        strict_reserved: opts.strict_reserved,
        dump_at_pc: None,
        dump_skip: 0,
        print_banner: false,
        runtime_max_steps: opts.max_steps,
        patch_bytes: &[],
        dump_mem_boot_addrs: &[],
        profile_pairs: false,
        budget_override: opts.budget_override,
        capture_state_trace: false,
        prescan: opts.prescan,
        guest_args: opts.guest_args,
    });
    let mut rt = prepared.rt;
    let authid_source = prepared.authid_source;
    let active_checkpoint = opts
        .checkpoint_override
        .unwrap_or_else(|| opts.title.checkpoint_trigger());
    super::run::configure_rsx_from_manifest(&mut rt, opts.title);

    let mut steps: usize = 0;
    let t0 = Instant::now();
    let outcome = bench_step_loop(&mut rt, active_checkpoint, &mut steps);
    let wall = t0.elapsed();

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
         event_queue_references={} event_queue_enqueues={} distinct_keys={}",
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

    BenchBootResult {
        steps,
        wall,
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
///
/// Each measurement runs in its own subprocess: in-process back-to-back
/// runs drift ~60 percent in wall time on Windows due to 1 GB
/// guest-memory page-commit reuse.
pub fn bench_boot_one_run(
    opts: BenchOptions<'_>,
    elf_data: Vec<u8>,
    authority_id: Option<u64>,
    control_flags1: Option<u32>,
) -> BenchBootResult {
    let r = bench_boot(opts, elf_data, authority_id, control_flags1);
    // wall_us, not wall_ms: the parent reconstructs the Duration from
    // this token, and millisecond truncation made the parent's
    // steps_per_sec disagree with the one printed here (and zeroed
    // the wall entirely on a sub-millisecond run).
    println!(
        "BENCH_RESULT steps={} wall_us={} steps_per_sec={:.0} outcome={}",
        r.steps,
        r.wall.as_micros(),
        r.steps_per_sec(),
        r.outcome,
    );
    r
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

/// Spawn the current binary as `bench-boot-once` and parse its
/// `BENCH_RESULT` line. Subprocess stderr is forwarded so warnings
/// reach the parent on the success path.
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
        Ok(r) => Ok((r, stderr)),
        Err(error) => Err(SpawnError::ParseFailed {
            error,
            stdout,
            stderr,
        }),
    }
}

/// Run [`bench_boot_one_run`] twice in separate subprocesses and
/// classify the pair against the gate.
pub fn bench_boot_pair(opts: BenchOptions<'_>) -> Result<BenchPairOutcome, SpawnError> {
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
        "bench-boot: title={} elf={} max_steps={}{overrides}",
        opts.title.name(),
        opts.elf_path,
        opts.max_steps
    );
    let (r1, r1_stderr) = spawn_one_run(opts)?;
    println!(
        "  run 1: steps={} wall_ms={:.3} steps_per_sec={:.0} outcome={}",
        r1.steps,
        r1.wall.as_secs_f64() * 1e3,
        r1.steps_per_sec(),
        r1.outcome,
    );
    let (r2, r2_stderr) = spawn_one_run(opts)?;
    println!(
        "  run 2: steps={} wall_ms={:.3} steps_per_sec={:.0} outcome={}",
        r2.steps,
        r2.wall.as_secs_f64() * 1e3,
        r2.steps_per_sec(),
        r2.outcome,
    );
    let drift_pct = wall_disagreement_percent(r1.wall, r2.wall);
    let witness_break = witness_disagreements(&r1_stderr, &r2_stderr);
    // Only run 1's stream reaches the anchor. That is sound only
    // because `witness_disagreements` above has already established
    // the two runs produced the same one.
    let anchor = if !opts.check_anchor {
        AnchorVerdict::Skipped
    } else {
        let reasons = incomparable_reasons(&opts);
        if reasons.is_empty() {
            check_anchor(
                &opts.title.content_id,
                r1.steps as u64,
                &r1.outcome.to_string(),
                &r1_stderr,
            )
        } else {
            AnchorVerdict::NotComparable(reasons)
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
        AnchorVerdict::NoBaseline => println!(
            "  anchor: none recorded for {} (skipped)",
            opts.title.content_id
        ),
        AnchorVerdict::Match => println!("  anchor: matches {}", opts.title.content_id),
        AnchorVerdict::Drift(f) => {
            println!(
                "  anchor: {} disagreement(s) vs {}",
                f.len(),
                opts.title.content_id
            )
        }
    }
    let gate = classify_pair(&r1, &r2, drift_pct, &witness_break, &anchor);
    match gate {
        BenchGate::Pass => {
            let d = drift_pct.expect("Pass implies finite drift");
            println!("  agreement: {d:.2}% (gate: <= {BENCH_AGREEMENT_GATE_PCT}% => OK)");
        }
        BenchGate::WallDriftExceeded => {
            let d = drift_pct.expect("WallDriftExceeded implies finite drift");
            println!("  agreement: {d:.2}% (gate: <= {BENCH_AGREEMENT_GATE_PCT}% => FAIL)");
        }
        BenchGate::WallUnmeasurable => {
            println!("  agreement: unmeasurable (gate: <= {BENCH_AGREEMENT_GATE_PCT}% => FAIL)");
        }
        BenchGate::DeterminismBreak => {
            println!("  agreement: determinism break");
            if r1.steps != r2.steps || r1.outcome != r2.outcome {
                println!("    steps/outcome differ between the two runs");
            }
            for failure in &witness_break {
                println!("    {failure}");
            }
        }
        BenchGate::AnchorDrift => {}
    }
    Ok(BenchPairOutcome {
        run1: r1,
        run2: r2,
        drift_pct,
        gate,
        anchor_failures: match anchor {
            AnchorVerdict::Drift(f) => f,
            _ => Vec::new(),
        },
    })
}

/// How a run compared against the title's committed anchor.
#[derive(Debug, Clone, PartialEq, Eq)]
enum AnchorVerdict {
    /// The check was not requested.
    Skipped,
    /// The comparison is meaningless for this invocation; each string
    /// names one cause. Distinct from [`Self::NoBaseline`] so a
    /// retargeted run never reads as "nothing is recorded".
    NotComparable(Vec<String>),
    /// No anchor is committed for this title, so there is nothing to
    /// compare against. A title being benchmarked before its first
    /// `record-anchors` is an ordinary state, not a failure.
    NoBaseline,
    /// The run reproduced every recorded value.
    Match,
    /// Every disagreement found, in the order the comparison made them.
    Drift(Vec<String>),
}

/// Why this invocation cannot be held against the committed anchor.
///
/// The anchor is measured by `record-anchors`, which boots the title
/// under its manifest defaults and nothing else. An override that
/// moves the trajectory yields a legitimately different run, so gating
/// it would report a regression that is not one. `--prescan` is absent
/// from the list because it only prints a decode report before
/// execution; `--firmware-dir` is absent because the resolved value
/// cannot be told apart from the auto-default here.
fn incomparable_reasons(opts: &BenchOptions<'_>) -> Vec<String> {
    let mut reasons = Vec::new();
    let recorded_cap = opts
        .title
        .bench_max_steps
        .unwrap_or(DEFAULT_BENCH_MAX_STEPS);
    if opts.max_steps as u64 != recorded_cap {
        reasons.push(format!(
            "--max-steps {} differs from the {recorded_cap} the anchor is recorded under",
            opts.max_steps
        ));
    }
    if let Some(cp) = opts.checkpoint_override {
        if cp != opts.title.checkpoint_trigger() {
            reasons.push(format!(
                "--checkpoint {} overrides the manifest checkpoint {}",
                cp.as_cli_str(),
                opts.title.checkpoint_trigger().as_cli_str()
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

/// Witness-level disagreements between the pair's two runs.
///
/// The steps/outcome comparison cannot see a counter that moved
/// without changing either, and the anchor check reads one run's
/// stream; without this, a witness that is nondeterministic across
/// runs passes the pair gate whenever it happens to match the anchor.
fn witness_disagreements(r1_stderr: &str, r2_stderr: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut parsed = Vec::new();
    for (label, stderr) in [("run 1", r1_stderr), ("run 2", r2_stderr)] {
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
        let present = if a.seen_lines.contains(line) { 1 } else { 2 };
        out.push(format!(
            "witness line {line} appeared in run {present} only"
        ));
    }
    for (name, x) in &a.values {
        let Some(y) = b.values.get(name) else {
            out.push(format!("witness {name}: run 1 {x}, absent from run 2"));
            continue;
        };
        if x != y {
            out.push(format!("witness {name}: run 1 {x} != run 2 {y}"));
        }
    }
    for name in b.values.keys() {
        if !a.values.contains_key(name) {
            out.push(format!(
                "witness {name}: run 2 {}, absent from run 1",
                b.values[name]
            ));
        }
    }
    out
}

/// Compare a run against a loaded anchor, returning every
/// disagreement.
///
/// Mirrors the comparison in `tests/title_witnesses.rs`: the two must
/// agree, or `bench-boot` would pass a run the witness suite rejects.
fn anchor_disagreements(
    baseline: &BootSummary,
    steps: u64,
    outcome: &str,
    observed: &ParsedWitnesses,
) -> Vec<String> {
    let mut failures = Vec::new();
    if steps != baseline.steps {
        failures.push(format!("steps {steps} != recorded {}", baseline.steps));
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

/// Load the anchor for `content_id` and compare `stderr`'s witness
/// lines against it.
///
/// An unreadable or unparseable anchor is a disagreement, not a skip:
/// only a genuinely absent file means "nothing recorded yet".
///
/// [`workspace_root`] is compiled in, so a binary invoked outside the
/// tree it was built from reaches no anchor at all. That says nothing
/// about what is recorded, so it reports as
/// [`AnchorVerdict::NotComparable`] rather than letting every title
/// look unrecorded.
fn check_anchor(content_id: &str, steps: u64, outcome: &str, stderr: &str) -> AnchorVerdict {
    check_anchor_under(&workspace_root(), content_id, steps, outcome, stderr)
}

fn check_anchor_under(
    root: &Path,
    content_id: &str,
    steps: u64,
    outcome: &str,
    stderr: &str,
) -> AnchorVerdict {
    if !root.is_dir() {
        return AnchorVerdict::NotComparable(vec![format!(
            "the compiled-in workspace root {} is not present on this machine, so no \
             committed anchor is reachable",
            root.display()
        )]);
    }
    let path = baseline_path(root, content_id);
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return AnchorVerdict::NoBaseline,
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
    let observed = match parse_witness_lines(stderr) {
        Ok(w) => w,
        Err(errs) => {
            return AnchorVerdict::Drift(
                errs.iter()
                    .map(|e| format!("malformed witness line: {e}"))
                    .collect(),
            );
        }
    };
    let failures = anchor_disagreements(&baseline, steps, outcome, &observed);
    if failures.is_empty() {
        AnchorVerdict::Match
    } else {
        AnchorVerdict::Drift(failures)
    }
}

/// Order matters: a determinism break makes the witness stream
/// meaningless, and an anchor disagreement outranks wall drift so a
/// contended host cannot mask a real regression behind a timing
/// failure.
fn classify_pair(
    r1: &BenchBootResult,
    r2: &BenchBootResult,
    drift_pct: Option<f64>,
    witness_break: &[String],
    anchor: &AnchorVerdict,
) -> BenchGate {
    if r1.steps != r2.steps || r1.outcome != r2.outcome || !witness_break.is_empty() {
        return BenchGate::DeterminismBreak;
    }
    if matches!(anchor, AnchorVerdict::Drift(_)) {
        return BenchGate::AnchorDrift;
    }
    match drift_pct {
        Some(d) if d > BENCH_AGREEMENT_GATE_PCT => BenchGate::WallDriftExceeded,
        Some(_) => BenchGate::Pass,
        None => BenchGate::WallUnmeasurable,
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
    #[error("BENCH_RESULT: missing steps= field")]
    MissingSteps,
    #[error("BENCH_RESULT: malformed steps={0:?}")]
    MalformedSteps(String),
    #[error("BENCH_RESULT: missing wall_us= field")]
    MissingWallUs,
    #[error("BENCH_RESULT: malformed wall_us={0:?}")]
    MalformedWallUs(String),
    #[error("BENCH_RESULT: missing outcome= field")]
    MissingOutcome,
    #[error("BENCH_RESULT: malformed outcome={token:?}: {source}")]
    UnparseableOutcome {
        token: String,
        #[source]
        source: BootOutcomeParseError,
    },
}

/// Parse the `BENCH_RESULT steps=N wall_us=M steps_per_sec=X outcome=O`
/// line out of captured stdout.
pub(crate) fn parse_bench_result(stdout: &str) -> Result<BenchBootResult, ParseBenchError> {
    let mut iter = stdout.lines().filter(|l| l.starts_with("BENCH_RESULT "));
    let line = iter.next().ok_or(ParseBenchError::NoResultLine)?;
    if iter.next().is_some() {
        return Err(ParseBenchError::DuplicateResultLine);
    }
    let mut steps: Option<usize> = None;
    let mut wall_us: Option<u64> = None;
    let mut outcome_token: Option<String> = None;
    let mut reported_sps: Option<f64> = None;
    for tok in line.split_whitespace().skip(1) {
        if let Some(v) = tok.strip_prefix("steps=") {
            steps = Some(
                v.parse()
                    .map_err(|_| ParseBenchError::MalformedSteps(v.to_string()))?,
            );
        } else if let Some(v) = tok.strip_prefix("wall_us=") {
            wall_us = Some(
                v.parse()
                    .map_err(|_| ParseBenchError::MalformedWallUs(v.to_string()))?,
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
    let steps = steps.ok_or(ParseBenchError::MissingSteps)?;
    let wall_us = wall_us.ok_or(ParseBenchError::MissingWallUs)?;
    let outcome_token = outcome_token.ok_or(ParseBenchError::MissingOutcome)?;
    let outcome = BootOutcome::from_str(&outcome_token).map_err(|source| {
        ParseBenchError::UnparseableOutcome {
            token: outcome_token.clone(),
            source,
        }
    })?;
    let wall = std::time::Duration::from_micros(wall_us);
    let result = BenchBootResult {
        steps,
        wall,
        outcome,
    };
    // Drift beyond rounding tolerance means the formatter and the
    // recomputation of (steps, wall_us) have gone out of sync. A zero
    // wall is unmeasurable, not drifted -- the pair gate rejects it
    // separately via `wall_disagreement_percent`.
    if let Some(reported) = reported_sps {
        if wall_us > 0 {
            let computed = result.steps_per_sec();
            let tolerance = (computed * 0.01).max(1.0);
            debug_assert!(
                (reported - computed).abs() <= tolerance,
                "BENCH_RESULT steps_per_sec drift: reported={reported} computed={computed} (tolerance={tolerance})"
            );
        }
    }
    Ok(result)
}

/// Relative wall-time disagreement between two runs, as a percentage
/// of the faster run. Returns `None` when either duration is zero so
/// the caller cannot silently pass an unmeasurable run through the
/// gate.
pub(crate) fn wall_disagreement_percent(
    a: std::time::Duration,
    b: std::time::Duration,
) -> Option<f64> {
    let aa = a.as_secs_f64();
    let bb = b.as_secs_f64();
    if !(aa > 0.0 && bb > 0.0) {
        return None;
    }
    let min = aa.min(bb);
    let max = aa.max(bb);
    Some(100.0 * (max - min) / min)
}

#[cfg(test)]
#[path = "tests/bench_tests.rs"]
mod tests;
