//! `run-game` subcommand: boot a PS3 ELF and drive the PPU step loop.

mod bench;
mod boot;
mod content;
mod diag;
pub mod manifest;
mod mounts;
mod observation;
mod prx;
mod stack_walk;
mod step_loop;

pub use bench::{bench_boot_one_run, bench_boot_pair};

pub(crate) use bench::agreement_percent;

use std::time::Instant;

use diag::{print_hle_summary, print_insn_coverage, print_shadow_stats, print_top_pcs};
use manifest::TitleManifest;
use observation::save_boot_observation;
use step_loop::{
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
    pub dump_at_pc: Option<u64>,
    pub dump_skip: u32,
    pub patch_bytes: &'a [(u64, u8)],
    pub dump_mem_boot_addrs: &'a [u64],
    pub dump_mem_fault_ranges: &'a [(u64, u64)],
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
        dump_mem_boot_addrs,
        dump_mem_fault_ranges,
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
        dump_mem_boot_addrs,
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
    if title.rsx_mirror() {
        rt.set_gcm_rsx_checkpoint(true);
        rt.set_rsx_mirror_writes(true);
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
    let (outcome, boot_outcome) = step_loop(&mut rt, &mut loop_ctx);
    let t_loop = t_loop_start.elapsed();
    let tty_oob_count = loop_ctx.tty_oob_count;
    let bogus_fd_count = loop_ctx.bogus_fd_count;

    println!("outcome: {outcome}");
    println!("steps: {steps}");
    // Must match the base set in boot::prepare.
    const HLE_HEAP_BASE: u32 = 0x10410000;
    let watermark = rt.hle_heap_watermark();
    let used = watermark.saturating_sub(HLE_HEAP_BASE);
    println!(
        "hle_heap_watermark: 0x{watermark:08x} ({used} bytes used above base 0x{HLE_HEAP_BASE:08x})"
    );
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
    print_hle_summary(&hle_calls, &hle_bindings);
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
        if let Err(msg) = save_boot_observation(
            path,
            &elf_data,
            rt.memory().as_bytes(),
            boot_outcome,
            steps,
            observation_manifest,
            rt.lv2_host().tty_log(),
        ) {
            eprintln!("save-observation: {msg}");
            std::process::exit(1);
        }
    }
}
