//! `run-game` subcommand: load a decrypted PS3 ELF and run the PPU
//! until fault, stall, or step limit.
//!
//! Split into submodules by responsibility:
//! - `ps3_layout`: PS3 guest-memory constants
//! - `bench`: bench-boot family (throughput measurement)
//! - `step_loop`: shared step-loop machinery (run-game diagnostic
//!   loop and bench minimal loop, plus the RSX-checkpoint
//!   classifier they both route through)
//! - `observation`: cross-runner checkpoint manifest + save
//! - `boot` / `diag` / `manifest` / `prx`: existing submodules

mod bench;
mod boot;
mod diag;
pub mod manifest;
mod observation;
mod prx;
mod ps3_layout;
mod step_loop;

pub use bench::{bench_boot_one_run, bench_boot_pair};

// Re-export for main.rs which reaches through `game::agreement_percent`.
pub(crate) use bench::agreement_percent;
pub(crate) use ps3_layout::{
    PS3_CHILD_STACKS_BASE, PS3_CHILD_STACKS_SIZE, PS3_PRIMARY_STACK_BASE, PS3_PRIMARY_STACK_SIZE,
    PS3_PRIMARY_STACK_TOP, PS3_RSX_BASE, PS3_RSX_SIZE, PS3_SPU_RESERVED_BASE,
    PS3_SPU_RESERVED_SIZE,
};

use std::time::Instant;

use diag::{print_hle_summary, print_insn_coverage, print_shadow_stats, print_top_pcs};
use manifest::TitleManifest;
use observation::save_boot_observation;
use step_loop::{
    compute_untracked, pct, step_loop, StepLoopCtx, StepTiming, PC_RING_SIZE, SYSCALL_RING_SIZE,
};

/// Tunables for one `run-game` invocation. Fields map directly
/// onto the equivalent CLI flags; the struct exists so the
/// function signature stays flat when a future flag lands (add a
/// field, one call-site line changes) and so `run_game`'s shape
/// matches `boot::PrepareOptions` already used in this crate.
pub struct RunGameOptions<'a> {
    pub title: &'a TitleManifest,
    pub elf_path: &'a str,
    pub max_steps: usize,
    pub trace: bool,
    pub profile: bool,
    pub firmware_dir: Option<&'a str>,
    pub dump_at_pc: Option<u64>,
    pub dump_skip: u32,
    pub patch_bytes: &'a [(u64, u8)],
    pub dump_mem_addrs: &'a [u64],
    pub save_observation: Option<&'a str>,
    pub observation_manifest: Option<&'a str>,
    pub strict_reserved: bool,
    pub profile_pairs: bool,
    pub budget_override: Option<u64>,
}

pub fn run_game(opts: RunGameOptions<'_>) {
    let RunGameOptions {
        title,
        elf_path,
        max_steps,
        trace,
        profile,
        firmware_dir,
        dump_at_pc,
        dump_skip,
        patch_bytes,
        dump_mem_addrs,
        save_observation,
        observation_manifest,
        strict_reserved,
        profile_pairs,
        budget_override,
    } = opts;
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
        budget_override,
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
        tty_oob_count: 0,
        bogus_fd_count: 0,
    };
    let (outcome, boot_outcome) = step_loop(&mut rt, &mut loop_ctx);
    let t_loop = t_loop_start.elapsed();
    let tty_oob_count = loop_ctx.tty_oob_count;
    let bogus_fd_count = loop_ctx.bogus_fd_count;

    println!("outcome: {outcome}");
    println!("steps: {steps}");
    // HLE heap watermark: peak address the bump allocator reached.
    // Subtract the base set in boot::prepare (0x10410000 today) to
    // get cumulative bytes leaked through the leak-on-free policy.
    // Reported unconditionally so any run becomes a data point for
    // sizing the arena and deciding when real individual-allocation
    // release becomes necessary (see `NID_SYS_FREE` TODO in
    // cellgov_core::hle::sys_prx_for_user).
    const HLE_HEAP_BASE: u32 = 0x10410000;
    let watermark = rt.hle_heap_watermark();
    let used = watermark.saturating_sub(HLE_HEAP_BASE);
    println!(
        "hle_heap_watermark: 0x{watermark:08x} ({used} bytes used above base 0x{HLE_HEAP_BASE:08x})"
    );
    // Report any reads that landed in a provisional RSX/SPU region. A
    // nonzero count surfaces silent zero-reads that would otherwise be
    // invisible at this scale.
    let prov = rt.memory().provisional_read_count();
    if prov > 0 {
        println!("provisional_reads: {prov} (reserved RSX/SPU regions returned zero)");
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
    print_hle_summary(&hle_calls, &hle_bindings);
    print_insn_coverage(&insn_coverage);
    print_top_pcs(&rt, &pc_hits);
    print_shadow_stats(&mut rt);

    if let Some(t) = &timing {
        println!();
        println!("profile:");
        if t_loop.is_zero() {
            // pct() clamps to 0.0 on zero total to avoid div-by-0,
            // which makes every percentage print as 0% and looks
            // identical to "nothing happened." Distinguish the two
            // with an explicit line so the operator does not
            // misread a clock-resolution artifact as a broken run.
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
        // Untracked = t_loop - (step + commit + coverage). Any
        // tracked-overflow surfaces as a WARN line rather than a
        // silent saturating_sub to zero.
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
        // Exit non-zero on save failure. A caller that asked for
        // --save-observation is almost always feeding a downstream
        // cross-runner comparison; reporting the write error but
        // exiting 0 would make the harness think the file exists.
        if let Err(msg) = save_boot_observation(
            path,
            &elf_data,
            rt.memory().as_bytes(),
            boot_outcome,
            steps,
            observation_manifest,
        ) {
            eprintln!("save-observation: {msg}");
            std::process::exit(1);
        }
    }
}
