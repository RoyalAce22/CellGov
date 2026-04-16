//! `run-game` subcommand: load a decrypted PS3 ELF and run the PPU
//! until fault, stall, or step limit.

mod boot;
mod diag;
pub mod manifest;
mod prx;

use diag::{
    fetch_raw_at, format_fault, format_max_steps, format_process_exit, print_hle_summary,
    print_insn_coverage, print_top_pcs, print_trace_line, ProcessExitInfo, TtyCapture,
};

use std::time::Instant;

use cellgov_core::{Runtime, StepError};

use manifest::TitleManifest;

/// PS3 LV2 primary-thread stack base. Matches RPCS3 `vm.cpp`'s
/// 0xD0000000 page-4K stack block.
pub(crate) const PS3_PRIMARY_STACK_BASE: u64 = 0xD000_0000;
/// Primary-thread stack size. 64 KB covers the default `SYS_PROCESS_PARAM`
/// stacksize for simple PS3 titles and all CellGov microtests.
pub(crate) const PS3_PRIMARY_STACK_SIZE: usize = 0x0001_0000;
/// Highest address reserved 16 bytes below the stack top, matching the
/// PPC64 ABI's requirement for a backchain+linkage area at the frame
/// boundary. `state.gpr[1]` is set to this value on thread entry.
pub(crate) const PS3_PRIMARY_STACK_TOP: u64 =
    PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64 - 0x10;
/// PS3 RSX video/local-memory base (`0xC0000000`). Reserved
/// placeholder; reads return zero, writes fault. Real RSX semantics
/// are out of scope here.
pub(crate) const PS3_RSX_BASE: u64 = 0xC000_0000;
/// RSX reservation size (256 MB) per RPCS3 `vm.cpp`.
pub(crate) const PS3_RSX_SIZE: usize = 0x1000_0000;
/// PS3 SPU-shared / reserved base (`0xE0000000`). Same semantics as
/// the RSX placeholder.
pub(crate) const PS3_SPU_RESERVED_BASE: u64 = 0xE000_0000;
/// SPU reservation size (512 MB) per RPCS3 `vm.cpp`.
pub(crate) const PS3_SPU_RESERVED_SIZE: usize = 0x2000_0000;

#[allow(clippy::too_many_arguments)]
pub fn run_game(
    title: &TitleManifest,
    elf_path: &str,
    max_steps: usize,
    trace: bool,
    profile: bool,
    firmware_dir: Option<&str>,
    dump_at_pc: Option<u64>,
    dump_skip: u32,
    patch_bytes: &[(u64, u8)],
    dump_mem_addrs: &[u64],
    save_observation: Option<&str>,
    observation_manifest: Option<&str>,
    strict_reserved: bool,
    profile_pairs: bool,
) {
    eprintln!(
        "run-game: title = {} ({})",
        title.name(),
        title.display_name()
    );
    let prepared = boot::prepare(boot::PrepareOptions {
        title,
        elf_path,
        firmware_dir,
        strict_reserved,
        dump_at_pc,
        dump_skip,
        module_start_max_steps: max_steps,
        print_banner: true,
        runtime_max_steps: max_steps,
        patch_bytes,
        dump_mem_addrs,
        profile_pairs,
    });
    let boot::PreparedBoot {
        mut rt,
        hle_bindings,
        elf_data,
        timings: st,
        ..
    } = prepared;

    if title.checkpoint_trigger() == manifest::CheckpointTrigger::FirstRsxWrite {
        rt.set_gcm_rsx_checkpoint(true);
    }

    if profile {
        println!("startup timing:");
        println!("  file read + mem alloc: {:?}", st.mem_alloc);
        println!("  ELF load:             {:?}", st.elf_load);
        println!("  HLE bind:             {:?}", st.hle_bind);
        println!("  PRX load + resolve:   {:?}", st.prx_load);
        println!("  total startup:        {:?}", st.total());
        println!();
    }

    let mut steps: usize = 0;
    let mut distinct_pcs: std::collections::BTreeSet<u64> = std::collections::BTreeSet::new();
    let mut hle_calls: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    let mut insn_coverage: std::collections::BTreeMap<&'static str, usize> =
        std::collections::BTreeMap::new();
    let mut timing = if profile {
        Some(StepTiming::default())
    } else {
        None
    };

    let mut pc_hits: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
    let t_loop_start = Instant::now();
    let mut loop_ctx = StepLoopCtx {
        steps: &mut steps,
        distinct_pcs: &mut distinct_pcs,
        hle_calls: &mut hle_calls,
        insn_coverage: &mut insn_coverage,
        hle_bindings: &hle_bindings,
        trace,
        timing: &mut timing,
        loop_start: t_loop_start,
        pc_ring: [0; PC_RING_SIZE],
        pc_ring_pos: 0,
        last_tty: None,
        last_exit: None,
        syscall_ring: [(0, 0); SYSCALL_RING_SIZE],
        syscall_ring_pos: 0,
        pc_hits: &mut pc_hits,
        checkpoint: title.checkpoint_trigger(),
    };
    let (outcome, boot_outcome) = step_loop(&mut rt, &mut loop_ctx);
    let t_loop = t_loop_start.elapsed();

    println!("outcome: {outcome}");
    println!("steps: {steps}");
    // Report any reads that landed in a provisional RSX/SPU region. A
    // nonzero count surfaces silent zero-reads that would otherwise be
    // invisible at this scale.
    let prov = rt.memory().provisional_read_count();
    if prov > 0 {
        println!("provisional_reads: {prov} (reserved RSX/SPU regions returned zero)");
    }
    print_hle_summary(&hle_calls, &hle_bindings);
    print_insn_coverage(&insn_coverage);
    print_top_pcs(&rt, &pc_hits);

    if let Some(t) = &timing {
        println!();
        println!("profile:");
        println!("  total loop:    {:?}", t_loop);
        println!(
            "  step (sched):  {:?}  ({:.1}%)",
            t.step_time,
            pct(t.step_time, t_loop)
        );
        println!(
            "  commit:        {:?}  ({:.1}%)",
            t.commit_time,
            pct(t.commit_time, t_loop)
        );
        println!(
            "  coverage tally:{:?}  ({:.1}%)",
            t.coverage_time,
            pct(t.coverage_time, t_loop)
        );
        let overhead = t_loop
            .saturating_sub(t.step_time)
            .saturating_sub(t.commit_time)
            .saturating_sub(t.coverage_time);
        println!(
            "  other overhead:{:?}  ({:.1}%)",
            overhead,
            pct(overhead, t_loop)
        );
        println!(
            "  steps/sec:     {:.0}",
            steps as f64 / t_loop.as_secs_f64()
        );
    }

    if profile_pairs {
        eprintln!();
        eprintln!("--- instruction frequency (raw decoded, top 40) ---");
        for (_, unit) in rt.registry_mut().iter_mut() {
            let insns = unit.drain_profile_insns();
            let total: u64 = insns.iter().map(|(_, c)| c).sum();
            for (name, count) in insns.iter().take(40) {
                eprintln!(
                    "  {:>12}  {:.2}%  {}",
                    count,
                    *count as f64 / total as f64 * 100.0,
                    name
                );
            }
        }
        eprintln!();
        eprintln!("--- adjacent pair frequency (raw decoded, top 40) ---");
        for (_, unit) in rt.registry_mut().iter_mut() {
            let pairs = unit.drain_profile_pairs();
            let total: u64 = pairs.iter().map(|(_, c)| c).sum();
            for ((a, b), count) in pairs.iter().take(40) {
                eprintln!(
                    "  {:>12}  {:.2}%  {} ; {}",
                    count,
                    *count as f64 / total as f64 * 100.0,
                    a,
                    b
                );
            }
        }
    }

    if let Some(path) = save_observation {
        save_boot_observation(
            path,
            &elf_data,
            rt.memory().as_bytes(),
            boot_outcome,
            steps,
            observation_manifest,
        );
    }
}

/// Result of one [`bench_boot`] invocation. Reports only what the
/// reproducibility harness needs: how many steps ran, how long the
/// step loop took, and how the boot terminated.
#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub struct BenchBootResult {
    pub steps: usize,
    pub wall: std::time::Duration,
    pub outcome: cellgov_compare::BootOutcome,
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

/// Run one boot with the minimum step-loop bookkeeping needed to
/// detect termination. The companion to `run_game` for throughput
/// measurement: no per-step HashMap entry, no BTreeSet insert, no
/// decode-again-for-coverage, no progress checkpoint. The boot setup
/// is byte-identical; only the step loop differs.
pub fn bench_boot(
    title: &TitleManifest,
    elf_path: &str,
    max_steps: usize,
    firmware_dir: Option<&str>,
    strict_reserved: bool,
    checkpoint_override: Option<manifest::CheckpointTrigger>,
) -> BenchBootResult {
    let prepared = boot::prepare(boot::PrepareOptions {
        title,
        elf_path,
        firmware_dir,
        strict_reserved,
        dump_at_pc: None,
        dump_skip: 0,
        module_start_max_steps: max_steps,
        print_banner: false,
        runtime_max_steps: max_steps,
        patch_bytes: &[],
        dump_mem_addrs: &[],
        profile_pairs: false,
    });
    let mut rt = prepared.rt;
    let checkpoint = checkpoint_override.unwrap_or_else(|| title.checkpoint_trigger());
    if checkpoint == manifest::CheckpointTrigger::FirstRsxWrite {
        rt.set_gcm_rsx_checkpoint(true);
    }

    let mut steps: usize = 0;
    let t0 = Instant::now();
    let outcome = bench_step_loop(&mut rt, checkpoint, &mut steps);
    let wall = t0.elapsed();

    BenchBootResult {
        steps,
        wall,
        outcome,
    }
}

/// Minimal step loop: only the state needed to detect a termination
/// condition. A ProcessExit fires when the runtime reports no
/// runnable unit (the `sys_process_exit` path removes the primary
/// unit); a FirstRsxWrite fires when `commit_step` returns
/// `ReservedWrite` into the rsx region; a fault breaks with Fault;
/// and exhausting `max_steps` breaks with MaxSteps.
fn bench_step_loop(
    rt: &mut Runtime,
    checkpoint: manifest::CheckpointTrigger,
    steps: &mut usize,
) -> cellgov_compare::BootOutcome {
    use cellgov_compare::BootOutcome;
    use manifest::CheckpointTrigger;
    let target_pc = match checkpoint {
        CheckpointTrigger::Pc(addr) => Some(addr),
        _ => None,
    };
    loop {
        match rt.step() {
            Ok(step) => {
                *steps += 1;
                let commit_result = rt.commit_step(&step.result, &step.effects);
                if rsx_write_checkpoint_addr(checkpoint, &commit_result).is_some() {
                    return BootOutcome::RsxWriteCheckpoint;
                }
                if let Some(target) = target_pc {
                    if step.result.local_diagnostics.pc == Some(target) {
                        return BootOutcome::PcReached(target);
                    }
                }
                if step.result.fault.is_some() {
                    return BootOutcome::Fault;
                }
            }
            Err(StepError::NoRunnableUnit) => return BootOutcome::ProcessExit,
            Err(StepError::MaxStepsExceeded) => return BootOutcome::MaxSteps,
            Err(StepError::TimeOverflow) => return BootOutcome::Fault,
        }
    }
}

/// Run a single bench invocation and print one machine-parseable
/// result line to stdout. Used as the inner call when
/// `bench_boot_pair` spawns a subprocess per measurement.
///
/// The subprocess-per-measurement shape is deliberate: running two
/// back-to-back measurements in the same process sees ~60 percent
/// wall-time drift between run 1 and run 2 on Windows, dominated by
/// 1 GB guest-memory allocation / page-commit reuse patterns. Each
/// measurement needs a fresh heap, fresh page tables, and fresh CPU
/// caches to be comparable.
pub fn bench_boot_one_run(
    title: &TitleManifest,
    elf_path: &str,
    max_steps: usize,
    firmware_dir: Option<&str>,
    strict_reserved: bool,
    checkpoint_override: Option<manifest::CheckpointTrigger>,
) -> BenchBootResult {
    let r = bench_boot(
        title,
        elf_path,
        max_steps,
        firmware_dir,
        strict_reserved,
        checkpoint_override,
    );
    println!(
        "BENCH_RESULT steps={} wall_ms={} steps_per_sec={:.0} outcome={}",
        r.steps,
        r.wall.as_millis(),
        r.steps_per_sec(),
        format_bench_outcome(r.outcome),
    );
    r
}

/// Render a [`cellgov_compare::BootOutcome`] for a `BENCH_RESULT`
/// line. Kept in one place so the emit side and the
/// [`parse_bench_result`] side share the canonical string form for
/// every variant, including the `PcReached(0xADDR)` shape which
/// carries a payload.
pub(crate) fn format_bench_outcome(outcome: cellgov_compare::BootOutcome) -> String {
    use cellgov_compare::BootOutcome;
    match outcome {
        BootOutcome::ProcessExit => "ProcessExit".into(),
        BootOutcome::RsxWriteCheckpoint => "RsxWriteCheckpoint".into(),
        BootOutcome::Fault => "Fault".into(),
        BootOutcome::MaxSteps => "MaxSteps".into(),
        BootOutcome::PcReached(addr) => format!("PcReached(0x{addr:x})"),
    }
}

/// Run `bench_boot_one_run` twice in two subprocesses and print a
/// pair report with the agreement percentage between the two runs.
/// The harness rejects a pair whose wall times disagree by more
/// than 5 percent.
///
/// Each subprocess call re-executes the current binary with a
/// dedicated `bench-boot-once` subcommand so that the second
/// measurement gets a fresh heap and cache state (see
/// [`bench_boot_one_run`] for the rationale).
pub fn bench_boot_pair(
    title: &TitleManifest,
    elf_path: &str,
    max_steps: usize,
    firmware_dir: Option<&str>,
    strict_reserved: bool,
    checkpoint_override: Option<manifest::CheckpointTrigger>,
) -> (BenchBootResult, BenchBootResult) {
    let checkpoint_label = match checkpoint_override {
        Some(manifest::CheckpointTrigger::Pc(a)) => format!(" checkpoint=pc=0x{a:x}"),
        Some(manifest::CheckpointTrigger::ProcessExit) => " checkpoint=process-exit".to_string(),
        Some(manifest::CheckpointTrigger::FirstRsxWrite) => {
            " checkpoint=first-rsx-write".to_string()
        }
        None => String::new(),
    };
    println!(
        "bench-boot: title={} elf={elf_path} max_steps={max_steps}{checkpoint_label}",
        title.name()
    );
    let r1 = spawn_one_run(
        title,
        elf_path,
        max_steps,
        firmware_dir,
        strict_reserved,
        checkpoint_override,
    );
    println!(
        "  run 1: steps={} wall_ms={} steps_per_sec={:.0} outcome={}",
        r1.steps,
        r1.wall.as_millis(),
        r1.steps_per_sec(),
        format_bench_outcome(r1.outcome),
    );
    let r2 = spawn_one_run(
        title,
        elf_path,
        max_steps,
        firmware_dir,
        strict_reserved,
        checkpoint_override,
    );
    println!(
        "  run 2: steps={} wall_ms={} steps_per_sec={:.0} outcome={}",
        r2.steps,
        r2.wall.as_millis(),
        r2.steps_per_sec(),
        format_bench_outcome(r2.outcome),
    );
    let agreement = agreement_percent(r1.wall, r2.wall);
    let gate = if agreement <= 5.0 { "OK" } else { "FAIL" };
    println!("  agreement: {agreement:.2}% (gate: <= 5% => {gate})");
    (r1, r2)
}

/// Fork-and-exec the current binary to run one `bench-boot-once`
/// invocation; parse the `BENCH_RESULT` line from stdout.
///
/// Inherits the subprocess's stderr so TTS and startup chatter still
/// reach the user; only stdout is captured for the parseable line.
fn spawn_one_run(
    title: &TitleManifest,
    elf_path: &str,
    max_steps: usize,
    firmware_dir: Option<&str>,
    strict_reserved: bool,
    checkpoint_override: Option<manifest::CheckpointTrigger>,
) -> BenchBootResult {
    let exe = std::env::current_exe().expect("current_exe");
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("bench-boot-once")
        .arg("--title")
        .arg(title.name())
        .arg("--max-steps")
        .arg(max_steps.to_string());
    if let Some(d) = firmware_dir {
        cmd.arg("--firmware-dir").arg(d);
    }
    if strict_reserved {
        cmd.arg("--strict-reserved");
    }
    if let Some(cp) = checkpoint_override {
        let value = match cp {
            manifest::CheckpointTrigger::ProcessExit => "process-exit".to_string(),
            manifest::CheckpointTrigger::FirstRsxWrite => "first-rsx-write".to_string(),
            manifest::CheckpointTrigger::Pc(a) => format!("pc=0x{a:x}"),
        };
        cmd.arg("--checkpoint").arg(value);
    }
    cmd.arg(elf_path);
    let output = cmd.output().expect("subprocess runs");
    let stdout = String::from_utf8_lossy(&output.stdout);
    parse_bench_result(&stdout).unwrap_or_else(|| {
        eprintln!("bench-boot: subprocess did not emit BENCH_RESULT line");
        eprintln!("stdout:\n{stdout}");
        eprintln!("stderr:\n{}", String::from_utf8_lossy(&output.stderr));
        std::process::exit(3);
    })
}

/// Parse a `BENCH_RESULT steps=N wall_ms=M steps_per_sec=X outcome=O`
/// line out of captured stdout. Returns `None` if no such line is
/// present or if any required field is missing / malformed.
pub(crate) fn parse_bench_result(stdout: &str) -> Option<BenchBootResult> {
    let line = stdout.lines().find(|l| l.starts_with("BENCH_RESULT "))?;
    let mut steps: Option<usize> = None;
    let mut wall_ms: Option<u64> = None;
    let mut outcome: Option<cellgov_compare::BootOutcome> = None;
    for tok in line.split_whitespace().skip(1) {
        if let Some(v) = tok.strip_prefix("steps=") {
            steps = v.parse().ok();
        } else if let Some(v) = tok.strip_prefix("wall_ms=") {
            wall_ms = v.parse().ok();
        } else if let Some(v) = tok.strip_prefix("outcome=") {
            outcome = match v {
                "ProcessExit" => Some(cellgov_compare::BootOutcome::ProcessExit),
                "RsxWriteCheckpoint" => Some(cellgov_compare::BootOutcome::RsxWriteCheckpoint),
                "Fault" => Some(cellgov_compare::BootOutcome::Fault),
                "MaxSteps" => Some(cellgov_compare::BootOutcome::MaxSteps),
                other => {
                    // PcReached carries a hex payload: `PcReached(0xADDR)`.
                    // Keep the parse strict -- malformed payloads return
                    // None so a corrupted `BENCH_RESULT` line fails loudly.
                    if let Some(addr_hex) = other
                        .strip_prefix("PcReached(0x")
                        .and_then(|s| s.strip_suffix(')'))
                    {
                        u64::from_str_radix(addr_hex, 16)
                            .ok()
                            .map(cellgov_compare::BootOutcome::PcReached)
                    } else {
                        None
                    }
                }
            };
        }
    }
    Some(BenchBootResult {
        steps: steps?,
        wall: std::time::Duration::from_millis(wall_ms?),
        outcome: outcome?,
    })
}

/// Relative wall-time difference between two runs, as a percentage
/// of the faster run. Used as the reproducibility gate: two bench
/// invocations must agree within 5 percent.
pub(crate) fn agreement_percent(a: std::time::Duration, b: std::time::Duration) -> f64 {
    let aa = a.as_secs_f64();
    let bb = b.as_secs_f64();
    if aa == 0.0 || bb == 0.0 {
        return 0.0;
    }
    let min = aa.min(bb);
    let max = aa.max(bb);
    100.0 * (max - min) / min
}

/// One region in a checkpoint observation manifest, sharing the schema
/// used by `tools/rpcs3_to_observation/` and `tests/fixtures/NPUA80001_checkpoint.toml`.
#[derive(Debug, serde::Deserialize)]
struct CheckpointManifest {
    regions: Vec<CheckpointRegion>,
}

/// A single named region in a checkpoint observation manifest.
#[derive(Debug, serde::Deserialize)]
struct CheckpointRegion {
    name: String,
    #[serde(deserialize_with = "de_hex_u64")]
    addr: u64,
    #[serde(deserialize_with = "de_hex_u64")]
    size: u64,
}

/// Highest end address of any PT_LOAD segment whose vaddr falls in
/// the PS3 user-memory region `[0x00010000, 0x10000000)`. Segments
/// in higher regions (HLE metadata at `0x10000000+`) do not share
/// address space with `sys_memory_allocate`, so they do not push the
/// allocator base forward.
///
/// Returns 0 if no qualifying segments are present.
pub(super) fn elf_user_region_end(data: &[u8]) -> usize {
    const PT_LOAD: u32 = 1;
    fn u16_be(d: &[u8], o: usize) -> u16 {
        u16::from_be_bytes([d[o], d[o + 1]])
    }
    fn u32_be(d: &[u8], o: usize) -> u32 {
        u32::from_be_bytes([d[o], d[o + 1], d[o + 2], d[o + 3]])
    }
    fn u64_be(d: &[u8], o: usize) -> u64 {
        u64::from_be_bytes([
            d[o],
            d[o + 1],
            d[o + 2],
            d[o + 3],
            d[o + 4],
            d[o + 5],
            d[o + 6],
            d[o + 7],
        ])
    }
    if data.len() < 64 || data[0..4] != [0x7f, 0x45, 0x4c, 0x46] {
        return 0;
    }
    let phoff = u64_be(data, 32) as usize;
    let phentsize = u16_be(data, 54) as usize;
    let phnum = u16_be(data, 56) as usize;
    let mut max_end: usize = 0;
    for i in 0..phnum {
        let base = phoff + i * phentsize;
        if base + phentsize > data.len() {
            break;
        }
        if u32_be(data, base) != PT_LOAD {
            continue;
        }
        let p_vaddr = u64_be(data, base + 16) as usize;
        let p_memsz = u64_be(data, base + 40) as usize;
        if p_memsz == 0 {
            continue;
        }
        if (0x0001_0000..0x1000_0000).contains(&p_vaddr) {
            let end = p_vaddr + p_memsz;
            if end > max_end {
                max_end = end;
            }
        }
    }
    max_end
}

fn de_hex_u64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    use serde::Deserialize;
    let s = String::deserialize(d)?;
    let trimmed = s.strip_prefix("0x").unwrap_or(&s);
    u64::from_str_radix(trimmed, 16).map_err(serde::de::Error::custom)
}

/// Build a boot-checkpoint observation and serialize it as JSON.
///
/// Region list defaults to the ELF's PT_LOAD segments (one region per
/// segment, named `seg{index}_{ro|rw}`). When `manifest_path` is set,
/// the regions come from that TOML manifest instead -- this is how a
/// cross-runner comparison guarantees matching region names on both
/// sides (CellGov and RPCS3 read the same manifest).
fn save_boot_observation(
    path: &str,
    elf_data: &[u8],
    final_memory: &[u8],
    outcome: cellgov_compare::BootOutcome,
    steps: usize,
    manifest_path: Option<&str>,
) {
    let regions: Vec<cellgov_compare::RegionDescriptor> = match manifest_path {
        Some(mp) => match std::fs::read_to_string(mp)
            .map_err(|e| format!("read {mp}: {e}"))
            .and_then(|t| {
                toml::from_str::<CheckpointManifest>(&t).map_err(|e| format!("parse {mp}: {e}"))
            }) {
            Ok(m) => m
                .regions
                .into_iter()
                .map(|r| cellgov_compare::RegionDescriptor {
                    name: r.name,
                    addr: r.addr,
                    size: r.size,
                })
                .collect(),
            Err(e) => {
                eprintln!("save-observation: {e}");
                return;
            }
        },
        None => {
            let segments = match cellgov_ppu::loader::pt_load_segments(elf_data) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("save-observation: failed to enumerate PT_LOAD: {e:?}");
                    return;
                }
            };
            segments
                .iter()
                .map(|s| {
                    let kind = if s.writable { "rw" } else { "ro" };
                    cellgov_compare::RegionDescriptor {
                        name: format!("seg{}_{kind}", s.index),
                        addr: s.vaddr,
                        size: s.memsz,
                    }
                })
                .collect()
        }
    };
    let observation = cellgov_compare::observe_from_boot(final_memory, outcome, steps, &regions);
    match serde_json::to_string_pretty(&observation) {
        Ok(json) => {
            if let Err(e) = std::fs::write(path, json) {
                eprintln!("save-observation: write to {path} failed: {e}");
            } else {
                println!(
                    "observation: wrote {} regions covering {} bytes to {path}",
                    observation.memory_regions.len(),
                    observation
                        .memory_regions
                        .iter()
                        .map(|r| r.data.len())
                        .sum::<usize>(),
                );
            }
        }
        Err(e) => eprintln!("save-observation: serialize failed: {e}"),
    }
}

fn pct(part: std::time::Duration, total: std::time::Duration) -> f64 {
    if total.is_zero() {
        0.0
    } else {
        100.0 * part.as_secs_f64() / total.as_secs_f64()
    }
}

#[derive(Default)]
struct StepTiming {
    step_time: std::time::Duration,
    commit_time: std::time::Duration,
    coverage_time: std::time::Duration,
}

pub(super) const PC_RING_SIZE: usize = 64;

struct StepLoopCtx<'a> {
    steps: &'a mut usize,
    distinct_pcs: &'a mut std::collections::BTreeSet<u64>,
    hle_calls: &'a mut std::collections::BTreeMap<u32, usize>,
    insn_coverage: &'a mut std::collections::BTreeMap<&'static str, usize>,
    hle_bindings: &'a [cellgov_ppu::prx::HleBinding],
    trace: bool,
    timing: &'a mut Option<StepTiming>,
    loop_start: Instant,
    /// Ring buffer of recent PCs for mini-trace on fault.
    pc_ring: [u64; PC_RING_SIZE],
    pc_ring_pos: usize,
    /// Last TTY write buffer (raw bytes) for diagnostic artifact.
    last_tty: Option<TtyCapture>,
    /// Set when sys_process_exit is dispatched.
    last_exit: Option<ProcessExitInfo>,
    /// Ring buffer of recent LV2 syscall numbers for exit diagnostic.
    syscall_ring: [(u64, u64); SYSCALL_RING_SIZE],
    syscall_ring_pos: usize,
    /// Per-PC hit counts. Identifies busy-loop bodies when the run
    /// hits max-steps without faulting: the loop's PCs dominate the
    /// top entries.
    pc_hits: &'a mut std::collections::HashMap<u64, u64>,
    /// The boot checkpoint the harness is looking for. See
    /// [`manifest::CheckpointTrigger`]. Controls whether a
    /// reserved-region write is treated as a checkpoint reach or as
    /// a normal fault (the commit pipeline discards either way).
    checkpoint: manifest::CheckpointTrigger,
}

/// Classify a `commit_step` outcome as a checkpoint hit, if the
/// title's trigger is [`manifest::CheckpointTrigger::FirstRsxWrite`]
/// and the commit failed with a `ReservedWrite` to the RSX region,
/// or the GCM control register's put pointer changed from zero
/// (indicating the game submitted RSX commands).
///
/// Returns the triggering guest address when the checkpoint fires,
/// `None` otherwise. Pulled out as a free function so a unit test
/// can pin the detection shape without spinning up a full runtime.
fn rsx_write_checkpoint_addr(
    trigger: manifest::CheckpointTrigger,
    commit_result: &Result<cellgov_core::CommitOutcome, cellgov_core::CommitError>,
) -> Option<u64> {
    if trigger != manifest::CheckpointTrigger::FirstRsxWrite {
        return None;
    }
    if let Err(cellgov_core::CommitError::Memory(cellgov_mem::MemError::ReservedWrite {
        addr,
        region: "rsx",
    })) = commit_result
    {
        Some(*addr)
    } else {
        None
    }
}

pub(super) const SYSCALL_RING_SIZE: usize = 32;

fn step_loop(
    rt: &mut Runtime,
    ctx: &mut StepLoopCtx<'_>,
) -> (String, cellgov_compare::BootOutcome) {
    use cellgov_compare::BootOutcome;
    loop {
        let t0 = Instant::now();
        let step_result = rt.step();
        let t1 = Instant::now();

        match step_result {
            Ok(step) => {
                *ctx.steps += 1;

                if let Some(pc) = step.result.local_diagnostics.pc {
                    ctx.distinct_pcs.insert(pc);
                    ctx.pc_ring[ctx.pc_ring_pos % PC_RING_SIZE] = pc;
                    ctx.pc_ring_pos += 1;
                    *ctx.pc_hits.entry(pc).or_insert(0) += 1;
                }

                // Progress checkpoint every 10K steps.
                if (*ctx.steps).is_multiple_of(10_000) {
                    let elapsed = ctx.loop_start.elapsed();
                    println!(
                        "  [{:>6}] {:.1?} elapsed, {} distinct PCs, {} HLE calls",
                        ctx.steps,
                        elapsed,
                        ctx.distinct_pcs.len(),
                        ctx.hle_calls.values().sum::<usize>(),
                    );
                }

                // Tally instruction coverage from the PC.
                let t_cov_start = Instant::now();
                if let Some(pc) = step.result.local_diagnostics.pc {
                    if let Some(raw) = fetch_raw_at(rt, pc) {
                        let name = match cellgov_ppu::decode::decode(raw) {
                            Ok(insn) => insn.variant_name(),
                            Err(_) => "DECODE_ERROR",
                        };
                        *ctx.insn_coverage.entry(name).or_insert(0) += 1;
                    }
                }
                let t_cov_end = Instant::now();

                if ctx.trace {
                    print_trace_line(rt, &step.result, *ctx.steps, ctx.hle_bindings);
                }
                // Track HLE/LV2 calls and capture TTY/exit before commit.
                if let Some(args) = &step.result.syscall_args {
                    let pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    if args[0] >= 0x10000 {
                        let idx = (args[0] - 0x10000) as u32;
                        *ctx.hle_calls.entry(idx).or_insert(0) += 1;
                        // Detect sys_process_exit via HLE dispatch.
                        if let Some(binding) = ctx.hle_bindings.get(idx as usize) {
                            if binding.nid == 0xe6f2c1e7 {
                                ctx.last_exit = Some(ProcessExitInfo {
                                    code: args[1] as u32,
                                    call_pc: pc,
                                });
                            }
                        }
                    } else if args[0] == 403 {
                        // sys_tty_write: always capture the buffer.
                        let buf = args[2] as usize;
                        let len = (args[3] as usize).min(4096);
                        let m = rt.memory().as_bytes();
                        if buf + len <= m.len() {
                            let raw = m[buf..buf + len].to_vec();
                            let text = String::from_utf8_lossy(&raw);
                            let fd = args[1] as u32;
                            print!("  tty[fd={}]: {text}", fd);
                            if !text.ends_with('\n') {
                                println!();
                            }
                            ctx.last_tty = Some(TtyCapture {
                                fd,
                                raw_bytes: raw,
                                call_pc: pc,
                            });
                        }
                    } else if args[0] == 22 {
                        // sys_process_exit: capture exit code and PC.
                        ctx.last_exit = Some(ProcessExitInfo {
                            code: args[1] as u32,
                            call_pc: pc,
                        });
                    }
                    // Track all syscalls (HLE and LV2) in ring buffer.
                    ctx.syscall_ring[ctx.syscall_ring_pos % SYSCALL_RING_SIZE] = (args[0], pc);
                    ctx.syscall_ring_pos += 1;
                }

                let t2 = Instant::now();
                let commit_result = rt.commit_step(&step.result, &step.effects);
                let t3 = Instant::now();

                if let Some(addr) = rsx_write_checkpoint_addr(ctx.checkpoint, &commit_result) {
                    break (
                        format!(
                            "RSX_WRITE_CHECKPOINT at 0x{addr:x} after {} steps",
                            ctx.steps
                        ),
                        BootOutcome::RsxWriteCheckpoint,
                    );
                }

                if let Some(t) = ctx.timing.as_mut() {
                    t.step_time += t1 - t0;
                    t.commit_time += t3 - t2;
                    t.coverage_time += t_cov_end - t_cov_start;
                }

                if let Some(fault) = &step.result.fault {
                    break (
                        format_fault(
                            rt,
                            &step.result,
                            fault,
                            *ctx.steps,
                            &ctx.pc_ring,
                            ctx.pc_ring_pos,
                        ),
                        BootOutcome::Fault,
                    );
                }
            }
            Err(StepError::NoRunnableUnit) => {
                if let Some(ref exit) = ctx.last_exit {
                    break (
                        format_process_exit(
                            exit,
                            ctx.last_tty.as_ref(),
                            *ctx.steps,
                            &ctx.pc_ring,
                            ctx.pc_ring_pos,
                            &ctx.syscall_ring,
                            ctx.syscall_ring_pos,
                            ctx.hle_bindings,
                        ),
                        BootOutcome::ProcessExit,
                    );
                }
                break (
                    format!("STALL after {} steps", ctx.steps),
                    BootOutcome::Fault,
                );
            }
            Err(StepError::MaxStepsExceeded) => {
                break (
                    format_max_steps(
                        *ctx.steps,
                        &ctx.pc_ring,
                        ctx.pc_ring_pos,
                        &ctx.syscall_ring,
                        ctx.syscall_ring_pos,
                        ctx.hle_bindings,
                    ),
                    BootOutcome::MaxSteps,
                );
            }
            Err(StepError::TimeOverflow) => {
                break (
                    format!("TIME_OVERFLOW after {} steps", ctx.steps),
                    BootOutcome::Fault,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        elf_user_region_end, manifest::CheckpointTrigger, rsx_write_checkpoint_addr,
        CheckpointManifest, CheckpointRegion,
    };
    use cellgov_core::{CommitError, CommitOutcome};
    use cellgov_mem::MemError;

    #[test]
    fn rsx_checkpoint_fires_on_reserved_write_to_rsx() {
        let err: Result<CommitOutcome, CommitError> =
            Err(CommitError::Memory(MemError::ReservedWrite {
                addr: 0xC000_0040,
                region: "rsx",
            }));
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &err),
            Some(0xC000_0040)
        );
    }

    #[test]
    fn rsx_checkpoint_ignores_other_reserved_regions() {
        // Writes to the SPU-reserved region must not trigger the
        // RSX checkpoint; only the "rsx" label qualifies.
        let err: Result<CommitOutcome, CommitError> =
            Err(CommitError::Memory(MemError::ReservedWrite {
                addr: 0xE000_0000,
                region: "spu_reserved",
            }));
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &err),
            None
        );
    }

    #[test]
    fn rsx_checkpoint_inert_for_process_exit_trigger() {
        // Titles whose trigger is ProcessExit (e.g. flow) keep the
        // historical behavior: a ReservedWrite is a fault, not a
        // checkpoint. This keeps flow's outcome stable.
        let err: Result<CommitOutcome, CommitError> =
            Err(CommitError::Memory(MemError::ReservedWrite {
                addr: 0xC000_0040,
                region: "rsx",
            }));
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::ProcessExit, &err),
            None
        );
    }

    #[test]
    fn rsx_checkpoint_ignores_successful_commit() {
        let ok: Result<CommitOutcome, CommitError> = Ok(CommitOutcome::default());
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &ok),
            None
        );
    }

    #[test]
    fn rsx_checkpoint_ignores_non_memory_commit_errors() {
        let err: Result<CommitOutcome, CommitError> =
            Err(CommitError::PayloadLengthMismatch { effect_index: 0 });
        assert_eq!(
            rsx_write_checkpoint_addr(CheckpointTrigger::FirstRsxWrite, &err),
            None
        );
    }

    #[test]
    fn agreement_percent_is_zero_for_identical_durations() {
        use std::time::Duration;
        assert_eq!(
            super::agreement_percent(Duration::from_millis(1000), Duration::from_millis(1000)),
            0.0
        );
    }

    #[test]
    fn agreement_percent_is_relative_to_faster_run() {
        // A 100ms / 105ms pair reports exactly 5 percent; pins the
        // gate boundary so a unit change in the formula surfaces
        // here instead of flipping a real bench from pass to fail.
        use std::time::Duration;
        let pct = super::agreement_percent(Duration::from_millis(100), Duration::from_millis(105));
        assert!((pct - 5.0).abs() < 0.0001, "expected 5.0, got {pct}");
    }

    #[test]
    fn agreement_percent_is_symmetric() {
        use std::time::Duration;
        let a = super::agreement_percent(Duration::from_millis(200), Duration::from_millis(250));
        let b = super::agreement_percent(Duration::from_millis(250), Duration::from_millis(200));
        assert_eq!(a, b);
    }

    #[test]
    fn agreement_percent_returns_zero_on_empty_duration() {
        // Guards the divide-by-zero path when a bench reports a
        // wall_ms of 0 (which can happen on a pathologically fast
        // run_game or a broken measurement).
        use std::time::Duration;
        assert_eq!(
            super::agreement_percent(Duration::ZERO, Duration::from_millis(100)),
            0.0
        );
    }

    #[test]
    fn parse_bench_result_extracts_fields() {
        let stdout = "some preamble\nBENCH_RESULT steps=1402388 wall_ms=323 steps_per_sec=4342377 outcome=ProcessExit\ntrailing noise\n";
        let r = super::parse_bench_result(stdout).expect("parses");
        assert_eq!(r.steps, 1402388);
        assert_eq!(r.wall.as_millis(), 323);
        assert_eq!(r.outcome, cellgov_compare::BootOutcome::ProcessExit);
    }

    #[test]
    fn parse_bench_result_handles_rsx_checkpoint_outcome() {
        let stdout =
            "BENCH_RESULT steps=12345 wall_ms=77 steps_per_sec=160000 outcome=RsxWriteCheckpoint\n";
        let r = super::parse_bench_result(stdout).expect("parses");
        assert_eq!(r.outcome, cellgov_compare::BootOutcome::RsxWriteCheckpoint);
    }

    #[test]
    fn parse_bench_result_handles_pc_reached_outcome() {
        let stdout = "BENCH_RESULT steps=1402388 wall_ms=250 steps_per_sec=5609552 outcome=PcReached(0x10381ce8)\n";
        let r = super::parse_bench_result(stdout).expect("parses");
        assert_eq!(
            r.outcome,
            cellgov_compare::BootOutcome::PcReached(0x10381ce8)
        );
        assert_eq!(r.steps, 1402388);
    }

    #[test]
    fn parse_bench_result_none_on_malformed_pc_reached() {
        // Missing the 0x prefix and trailing paren; our parser
        // refuses to guess and returns None so a corrupted line
        // fails loudly at the call site.
        let stdout = "BENCH_RESULT steps=1 wall_ms=1 steps_per_sec=1 outcome=PcReached(abc\n";
        assert!(super::parse_bench_result(stdout).is_none());
    }

    #[test]
    fn format_bench_outcome_pc_reached_hex() {
        let s = super::format_bench_outcome(cellgov_compare::BootOutcome::PcReached(0x10381ce8));
        assert_eq!(s, "PcReached(0x10381ce8)");
    }

    #[test]
    fn parse_bench_result_none_when_no_result_line() {
        let stdout = "just some noise\nbut no result line\n";
        assert!(super::parse_bench_result(stdout).is_none());
    }

    #[test]
    fn parse_bench_result_none_on_unknown_outcome() {
        let stdout = "BENCH_RESULT steps=1 wall_ms=1 steps_per_sec=1 outcome=WhoKnows\n";
        assert!(super::parse_bench_result(stdout).is_none());
    }

    /// Build a minimal big-endian ELF64 header with N PT_LOAD program
    /// headers at the supplied (vaddr, memsz) tuples. Just enough
    /// structure for `elf_user_region_end` to scan -- the segments'
    /// payloads are not present.
    fn synthetic_elf(loads: &[(u64, u64)]) -> Vec<u8> {
        let phoff: u64 = 64;
        let phentsize: u16 = 56;
        let phnum: u16 = loads.len() as u16;
        let header_end = phoff as usize + phentsize as usize * phnum as usize;
        let mut buf = vec![0u8; header_end];
        buf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
        buf[4] = 2; // ELFCLASS64
        buf[5] = 2; // ELFDATA2MSB (big-endian)
        buf[32..40].copy_from_slice(&phoff.to_be_bytes());
        buf[54..56].copy_from_slice(&phentsize.to_be_bytes());
        buf[56..58].copy_from_slice(&phnum.to_be_bytes());
        for (i, &(vaddr, memsz)) in loads.iter().enumerate() {
            let base = phoff as usize + i * phentsize as usize;
            buf[base..base + 4].copy_from_slice(&1u32.to_be_bytes()); // PT_LOAD
            buf[base + 16..base + 24].copy_from_slice(&vaddr.to_be_bytes());
            buf[base + 40..base + 48].copy_from_slice(&memsz.to_be_bytes());
        }
        buf
    }

    #[test]
    fn elf_user_region_end_picks_max_in_user_range() {
        // Two PT_LOAD segments inside the user-memory range; the
        // helper must return the highest end address among them so
        // the allocator base lands above the loaded ELF.
        let elf = synthetic_elf(&[(0x0001_0000, 0x80_0000), (0x0082_0000, 0x7_5CD4)]);
        assert_eq!(elf_user_region_end(&elf), 0x0082_0000 + 0x7_5CD4);
    }

    #[test]
    fn elf_user_region_end_ignores_segments_above_user_range() {
        // Segments at 0x10000000+ (HLE PT_LOADs) do not push the
        // allocator base forward -- they live in a separate VA range
        // and do not share the user-memory pool.
        let elf = synthetic_elf(&[
            (0x0001_0000, 0x10_0000),
            (0x1000_0000, 0x4_0000),
            (0x1006_0000, 0x100),
        ]);
        assert_eq!(elf_user_region_end(&elf), 0x0001_0000 + 0x10_0000);
    }

    #[test]
    fn elf_user_region_end_skips_zero_memsz() {
        let elf = synthetic_elf(&[(0x0001_0000, 0), (0x0002_0000, 0x100)]);
        assert_eq!(elf_user_region_end(&elf), 0x0002_0000 + 0x100);
    }

    #[test]
    fn elf_user_region_end_returns_zero_for_no_user_segments() {
        // All segments outside the user range -> nothing to push.
        let elf = synthetic_elf(&[(0x1000_0000, 0x4_0000)]);
        assert_eq!(elf_user_region_end(&elf), 0);
    }

    #[test]
    fn elf_user_region_end_rejects_non_elf_input() {
        assert_eq!(elf_user_region_end(&[0u8; 64]), 0);
        assert_eq!(elf_user_region_end(&[0u8; 4]), 0);
    }

    fn parse(text: &str) -> CheckpointManifest {
        toml::from_str(text).expect("parses")
    }

    #[test]
    fn checkpoint_manifest_parses_hex_addresses() {
        // The run-game --observation-manifest flag uses this struct
        // to override the PT_LOAD-derived region list. Hex parsing
        // must accept the same syntax the rpcs3_to_observation
        // adapter accepts so both runners read the same manifest
        // file.
        let m = parse(
            r#"
            [[regions]]
            name = "code"
            addr = "0x10000"
            size = "0x800000"

            [[regions]]
            name = "rodata"
            addr = "0x10000000"
            size = "0x40000"
            "#,
        );
        assert_eq!(m.regions.len(), 2);
        let CheckpointRegion {
            ref name,
            addr,
            size,
        } = m.regions[0];
        assert_eq!(name, "code");
        assert_eq!(addr, 0x10000);
        assert_eq!(size, 0x800000);
        assert_eq!(m.regions[1].addr, 0x1000_0000);
        assert_eq!(m.regions[1].size, 0x40000);
    }

    #[test]
    fn checkpoint_manifest_accepts_unprefixed_hex() {
        // The deserializer strips an optional 0x prefix so manifests
        // can be hand-edited either way.
        let m = parse(
            r#"
            [[regions]]
            name = "r"
            addr = "1000"
            size = "10"
            "#,
        );
        assert_eq!(m.regions[0].addr, 0x1000);
        assert_eq!(m.regions[0].size, 0x10);
    }

    #[test]
    fn checkpoint_manifest_rejects_non_hex_value() {
        let bad = toml::from_str::<CheckpointManifest>(
            r#"
            [[regions]]
            name = "r"
            addr = "not-hex"
            size = "10"
            "#,
        );
        assert!(bad.is_err(), "non-hex addr must fail");
    }

    #[test]
    fn checkpoint_manifest_loads_committed_flow_fixture() {
        // Pin the checked-in flOw checkpoint manifest so a future
        // edit that breaks parsing fails locally and not in a live
        // cross-runner run.
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("tests")
            .join("fixtures")
            .join("NPUA80001_checkpoint.toml");
        let text = std::fs::read_to_string(&path).expect("read");
        let m: CheckpointManifest = toml::from_str(&text).expect("parses");
        assert!(!m.regions.is_empty());
        assert!(m.regions.iter().any(|r| r.name == "code"));
    }
}
