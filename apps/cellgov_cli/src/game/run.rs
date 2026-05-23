//! `run-game` entry point and its supporting option / summary / error
//! types. Public surface re-exported from [`super`].

use std::time::Instant;

use cellgov_compare::BootOutcome;
use cellgov_core::Runtime;
use cellgov_time::Budget;

use crate::game::boot::{self, BootMode};
use crate::game::diag::{
    print_hle_summary, print_insn_coverage, print_shadow_stats, print_top_pcs,
};
use crate::game::manifest::TitleManifest;
use crate::game::observation::{self, save_boot_observation};
use crate::game::step_loop::{
    compute_untracked, pct, step_loop, RingCursor, StepLoopCtx, StepTiming, PC_RING_SIZE,
    SYSCALL_RING_SIZE,
};

pub struct RunGameOptions<'a> {
    pub title: &'a TitleManifest,
    pub elf_path: &'a str,
    pub max_steps: usize,
    pub trace: bool,
    pub profile: bool,
    pub firmware_dir: Option<&'a str>,
    pub boot_mode: BootMode,
    pub dump_at_pc: Option<u64>,
    pub dump_skip: u32,
    pub patch_bytes: &'a [(u64, u8)],
    pub dump_mem_boot_addrs: &'a [u64],
    pub dump_mem_fault_ranges: &'a [(u64, u64)],
    pub save_observation: Option<&'a str>,
    pub observation_manifest: Option<&'a str>,
    pub save_boot_summary: Option<&'a str>,
    pub save_state_trace: Option<&'a str>,
    pub strict_reserved: bool,
    pub profile_pairs: bool,
    pub budget_override: Option<Budget>,
}

/// Terminal-state summary from [`run_game`], shaped for the CLI's
/// exit-code gate.
pub struct RunSummary {
    pub outcome: BootOutcome,
    pub had_critical_anomaly: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("save-observation: {0}")]
    SaveObservation(#[source] observation::ObservationSaveError),
    #[error("save-boot-summary: {0}")]
    SaveBootSummary(#[source] observation::ObservationSaveError),
}

/// Apply the manifest-driven RSX init toggles.
pub(in crate::game) fn configure_rsx_from_manifest(rt: &mut Runtime, title: &TitleManifest) {
    if title.rsx_mirror() {
        rt.set_rsx_mirror_writes(true);
    }
}

/// Boot a PS3 ELF and drive the PPU step loop until a terminal state.
///
/// # Errors
///
/// Returns [`RunError::SaveObservation`] when `--save-observation`
/// was requested but writing the JSON failed.
pub fn run_game(opts: RunGameOptions<'_>) -> Result<RunSummary, RunError> {
    let RunGameOptions {
        title,
        elf_path,
        max_steps,
        trace,
        profile,
        firmware_dir,
        boot_mode,
        dump_at_pc,
        dump_skip,
        patch_bytes,
        dump_mem_boot_addrs,
        dump_mem_fault_ranges,
        save_observation,
        observation_manifest,
        save_boot_summary,
        save_state_trace,
        strict_reserved,
        profile_pairs,
        budget_override,
    } = opts;
    // The CLI parser enforces these; the asserts catch direct
    // `RunGameOptions` construction that bypasses parsing.
    for (i, &(addr, len)) in dump_mem_fault_ranges.iter().enumerate() {
        debug_assert!(
            len > 0,
            "dump_mem_fault_ranges[{i}]: zero length at addr 0x{addr:x}"
        );
        debug_assert!(
            addr.checked_add(len.saturating_sub(1)).is_some(),
            "dump_mem_fault_ranges[{i}]: addr 0x{addr:x} + len 0x{len:x} overflows u64"
        );
    }
    let profile_run = crate::cli::env::parse_env_bool("CELLGOV_RUNGAME_PROFILE");
    let t_run_start = Instant::now();
    eprintln!(
        "run-game: title = {} ({})",
        title.name(),
        title.display_name()
    );
    let prepared = boot::prepare(boot::PrepareOptions {
        title,
        elf_path,
        firmware_dir,
        boot_mode,
        strict_reserved,
        dump_at_pc,
        dump_skip,
        module_start_max_steps: max_steps,
        print_banner: true,
        runtime_max_steps: max_steps,
        patch_bytes,
        dump_mem_boot_addrs,
        profile_pairs,
        budget_override,
        capture_state_trace: save_state_trace.is_some(),
    });
    let t_after_prepare = Instant::now();
    let boot::PreparedBoot {
        mut rt,
        elf_data,
        timings: st,
        step_budget,
        ..
    } = prepared;

    configure_rsx_from_manifest(&mut rt, title);

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

    let mut pc_hits: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();
    let mut loop_ctx = StepLoopCtx {
        steps: &mut steps,
        distinct_pcs: &mut distinct_pcs,
        hle_calls: &mut hle_calls,
        insn_coverage: &mut insn_coverage,
        trace,
        timing: &mut timing,
        loop_start: Instant::now(),
        pc_ring: [0; PC_RING_SIZE],
        pc_ring_cursor: RingCursor::new(PC_RING_SIZE),
        last_tty: None,
        last_exit: None,
        syscall_ring: [(0, 0); SYSCALL_RING_SIZE],
        syscall_ring_cursor: RingCursor::new(SYSCALL_RING_SIZE),
        pc_hits: &mut pc_hits,
        checkpoint: title.checkpoint_trigger(),
        tty_oob_count: 0,
        bogus_fd_count: 0,
        dump_mem_fault_ranges,
    };
    let t_loop_start = Instant::now();
    loop_ctx.loop_start = t_loop_start;
    let (outcome, boot_outcome) = step_loop(&mut rt, &mut loop_ctx);
    let t_loop = t_loop_start.elapsed();
    let t_after_steploop = Instant::now();
    let dirty_pages_after_steploop = rt.memory().dirty_page_count();
    let tty_oob_count = loop_ctx.tty_oob_count;
    let bogus_fd_count = loop_ctx.bogus_fd_count;

    println!("outcome: {outcome}");
    println!("steps: {steps}");
    let prov = rt.memory().provisional_read_count();
    if prov > 0 {
        println!("provisional_reads: {prov} (reserved RSX/SPU regions returned zero)");
    }
    let displacements = rt.syscall_responses().displacement_count();
    if displacements > 0 {
        println!(
            "syscall_response_displacements: {displacements} (pending wake responses overwritten before drain; lost r3 + out-pointer writes)"
        );
    }
    if tty_oob_count > 0 {
        println!(
            "tty_oob_captures_dropped: {tty_oob_count} (sys_tty_write calls whose buffer overflowed guest memory)"
        );
    }
    if bogus_fd_count > 0 {
        println!(
            "tty_bogus_fd_calls: {bogus_fd_count} (sys_tty_write calls with fd values not fitting in u32)"
        );
    }
    print_hle_summary(&hle_calls);
    print_insn_coverage(&insn_coverage);
    print_top_pcs(&rt, &pc_hits);
    print_shadow_stats(&mut rt);

    if let Some(t) = &timing {
        println!();
        println!("profile:");
        if t_loop.is_zero() {
            println!(
                "  WARN: t_loop is zero (clock resolution artifact or instantaneous loop); percentages below are meaningless"
            );
        }
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
        match compute_untracked(t_loop, t.step_time, t.commit_time, t.coverage_time) {
            Ok(overhead) => {
                println!(
                    "  other overhead:{:?}  ({:.1}%)",
                    overhead,
                    pct(overhead, t_loop)
                );
            }
            Err(excess) => {
                println!(
                    "  other overhead: WARN tracked buckets exceed loop total by {:?}",
                    excess
                );
            }
        }
        if t_loop.is_zero() {
            println!("  steps/sec:     n/a (loop time below clock resolution)");
        } else {
            println!(
                "  steps/sec:     {:.0}",
                steps as f64 / t_loop.as_secs_f64()
            );
        }
    }

    if profile_pairs {
        for (id, unit) in rt.registry_mut().iter_mut() {
            let insns = unit.drain_profile_insns();
            let total: u64 = insns.iter().map(|(_, c)| c).sum();
            eprintln!();
            eprintln!(
                "--- unit {}: instruction frequency (raw decoded, top 40, total={total}) ---",
                id.raw()
            );
            for (name, count) in insns.iter().take(40) {
                eprintln!(
                    "  {:>12}  {:.2}%  {}",
                    count,
                    *count as f64 / total as f64 * 100.0,
                    name
                );
            }
        }
        for (id, unit) in rt.registry_mut().iter_mut() {
            let pairs = unit.drain_profile_pairs();
            let total: u64 = pairs.iter().map(|(_, c)| c).sum();
            eprintln!();
            eprintln!(
                "--- unit {}: adjacent pair frequency (raw decoded, top 40, total={total}) ---",
                id.raw()
            );
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
            rt.lv2_host().tty_log(),
        )
        .map_err(RunError::SaveObservation)?;
    }
    if let Some(path) = save_boot_summary {
        let host_invariant_breaks = rt.lv2_host().invariant_break_count() as u64;
        observation::save_boot_summary_json(
            path,
            title,
            boot_outcome,
            steps,
            step_budget,
            host_invariant_breaks,
        )
        .map_err(RunError::SaveBootSummary)?;
    }
    if let Some(path) = save_state_trace {
        let bytes = rt.trace().bytes();
        std::fs::write(path, bytes).unwrap_or_else(|e| {
            crate::cli::exit::die(&format!(
                "save-state-trace: failed to write {} ({} bytes): {e}",
                path,
                bytes.len(),
            ))
        });
        eprintln!("save-state-trace: wrote {} bytes to {path}", bytes.len());
    }
    let t_after_save = Instant::now();

    if profile_run {
        let prepare_ms = t_after_prepare.duration_since(t_run_start).as_secs_f64() * 1000.0;
        let steploop_ms = t_after_steploop
            .duration_since(t_after_prepare)
            .as_secs_f64()
            * 1000.0;
        let save_ms = t_after_save.duration_since(t_after_steploop).as_secs_f64() * 1000.0;
        let total_ms = t_after_save.duration_since(t_run_start).as_secs_f64() * 1000.0;
        eprintln!(
            "rungame_profile: prepare={prepare_ms:.2}ms steploop={steploop_ms:.2}ms \
             save={save_ms:.2}ms total={total_ms:.2}ms steps={steps} \
             dirty_pages_at_steploop_exit={dirty_pages_after_steploop}",
        );
    }

    Ok(RunSummary {
        outcome: boot_outcome,
        had_critical_anomaly: displacements > 0,
    })
}
