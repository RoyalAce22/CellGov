//! `run-game` subcommand: load a decrypted PS3 ELF and run the PPU
//! until fault, stall, or step limit.

mod diag;
mod prx;

use diag::{
    fetch_raw_at, format_fault, format_max_steps, format_process_exit, print_hle_summary,
    print_insn_coverage, print_top_pcs, print_trace_line, ProcessExitInfo, TtyCapture,
};
use prx::{build_nid_map, load_firmware_prx, pre_init_tls, run_module_start};

use std::time::Instant;

use cellgov_core::{Runtime, RuntimeMode, StepError};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

use super::{die, load_file_or_die};

#[allow(clippy::too_many_arguments)]
pub fn run_game(
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
) {
    let t_start = Instant::now();
    let elf_data = load_file_or_die(elf_path);

    // Determine memory size from ELF segments.
    let required_size = cellgov_ppu::loader::required_memory_size(&elf_data)
        .unwrap_or_else(|e| die(&format!("failed to parse ELF: {e:?}")));

    // Round up to 64KB alignment. Guest memory must be large enough for
    // the game, PRX modules, and kernel memory allocations (at 0x30000000+).
    // flOw's memory allocator lays its regions out past 0x31000000 (the
    // game's `r27 = mem_size` register is the upper boundary it uses as
    // base for several large sub-regions). Extending to 1 GB covers
    // the observed 0x31b00028 accesses with headroom without going to
    // the full PS3 virtual-address space.
    let min_for_kernel = 0x4000_0000usize; // covers kernel alloc region
    let game_size = ((required_size + 0xFFFF) & !0xFFFF) + 0x200000;
    let mem_size = game_size.max(min_for_kernel);
    let mut state = cellgov_ppu::state::PpuState::new();
    let mut mem = cellgov_mem::GuestMemory::new(mem_size);
    let t_mem_alloc = t_start.elapsed();

    // Address 0 must stay zero: CRT0 linked lists use *NULL as a
    // termination sentinel. No exit stub is planted here.

    let load_result = cellgov_ppu::loader::load_ppu_elf(&elf_data, &mut mem, &mut state)
        .unwrap_or_else(|e| die(&format!("failed to load ELF: {e:?}")));
    let t_elf_load = t_start.elapsed();

    // Parse and bind HLE import stubs.
    let tramp_base = ((required_size + 0xFFF) & !0xFFF) as u32;
    let hle_bindings = match cellgov_ppu::prx::parse_imports(&elf_data) {
        Ok(modules) => {
            let bindings = cellgov_ppu::prx::bind_hle_stubs(&modules, &mut mem, tramp_base);
            println!(
                "imports: {} modules, {} functions bound to HLE stubs",
                modules.len(),
                bindings.len()
            );
            for m in &modules {
                println!("  {}: {} functions", m.name, m.functions.len());
            }
            bindings
        }
        Err(e) => {
            println!("imports: none (parse failed: {e:?})");
            vec![]
        }
    };
    let t_hle_bind = t_start.elapsed();

    // Load firmware PRX module (liblv2) if a firmware directory is provided.
    // Real exports from the loaded module overwrite HLE GOT patches.
    let prx_info = load_firmware_prx(firmware_dir, &hle_bindings, &mut mem, tramp_base);
    let t_prx_load = t_start.elapsed();

    // Pre-initialize the TLS area before module_start. On real PS3, the
    // kernel sets up TLS from the game ELF's PT_TLS segment before any
    // module_start runs. We replicate this by copying the TLS template
    // and setting r13.
    pre_init_tls(&elf_data, &mut mem);

    // Execute module_start for the loaded PRX before the game.
    // This initializes libc state (guard variables, heap arenas, TLS)
    // that the game's CRT0 depends on.
    if let Some(ref info) = prx_info {
        mem = run_module_start(mem, info, &hle_bindings, max_steps);
    }

    // Set up stack
    let stack_top = (mem_size as u64) - 0x1000;
    state.gpr[1] = stack_top;
    state.lr = 0;

    // Entry registers matching RPCS3's ppu_load_exec. The game's CRT0
    // reads these to initialize libc's heap, TLS, and argv/envp.
    //   r3 = argc, r4 = argv, r5 = envp, r6 = envp count
    //   r7 = primary PPU thread id
    //   r8 = TLS vaddr, r9 = TLS filesize, r10 = TLS memsize
    //   r11 = ELF e_entry
    //   r12 = malloc_pagesize from .sys_proc_param (fallback 0x100000 = 1MB)
    let tls_info = cellgov_ppu::loader::find_tls_segment(&elf_data);
    state.gpr[3] = 0; // argc
    state.gpr[4] = 0; // argv
    state.gpr[5] = 0; // envp
    state.gpr[6] = 0; // envp count
    state.gpr[7] = 0x0100_0000; // primary PPU thread id (RPCS3 default)
    state.gpr[8] = tls_info.map(|t| t.vaddr).unwrap_or(0);
    state.gpr[9] = tls_info.map(|t| t.filesz).unwrap_or(0);
    state.gpr[10] = tls_info.map(|t| t.memsz).unwrap_or(0);
    state.gpr[11] = load_result.entry;
    let proc_param = cellgov_ppu::loader::find_sys_process_param(&elf_data);
    let malloc_pagesize = proc_param.map(|p| p.malloc_pagesize).unwrap_or(0x100000);
    state.gpr[12] = malloc_pagesize as u64;

    println!("elf: {elf_path}");
    println!("memory: {} MB", mem_size / (1024 * 1024));
    println!(
        "entry: 0x{:x} (OPD) -> pc=0x{:x} toc=0x{:x}",
        load_result.entry, state.pc, state.gpr[2]
    );
    if let Some(p) = proc_param {
        println!(
            "sys_proc_param: sdk=0x{:x} prio={} stack=0x{:x} malloc_pagesize=0x{:x}",
            p.sdk_version, p.primary_prio, p.primary_stacksize, p.malloc_pagesize,
        );
    } else {
        println!("sys_proc_param: not found, using malloc_pagesize=0x{malloc_pagesize:x}");
    }
    if let Some(ref info) = prx_info {
        println!(
            "prx: {} at 0x{:x} (toc=0x{:x}, {} relocs, {}/{} imports resolved)",
            info.name, info.base, info.toc, info.relocs_applied, info.resolved, info.total_imports,
        );
    }
    println!("max_steps: {max_steps}");
    println!();

    if profile {
        println!("startup timing:");
        println!("  file read + mem alloc: {:?}", t_mem_alloc);
        println!("  ELF load:             {:?}", t_elf_load - t_mem_alloc);
        println!("  HLE bind:             {:?}", t_hle_bind - t_elf_load);
        println!("  PRX load + resolve:   {:?}", t_prx_load - t_hle_bind);
        println!("  total startup:        {:?}", t_prx_load);
        println!();
    }

    // Apply any --patch-byte requests so investigative runs can override
    // specific flag bytes the game's static init depends on, before any
    // guest code observes the commit.
    for &(addr, val) in patch_bytes {
        if let Some(range) = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 1) {
            let _ = mem.apply_commit(range, &[val]);
            println!("patch: byte 0x{addr:x} = 0x{val:02x}");
        }
    }

    // --dump-mem: print 32 bytes at each requested address so investigative
    // runs can verify loader-initialized static data before any guest code
    // has had a chance to mutate it.
    for &addr in dump_mem_addrs {
        let bytes = mem.as_bytes();
        let a = addr as usize;
        if a + 32 <= bytes.len() {
            print!("mem[0x{addr:x}]:");
            for b in &bytes[a..a + 32] {
                print!(" {b:02x}");
            }
            println!();
        } else {
            println!("mem[0x{addr:x}]: out of range");
        }
    }

    // Budget=1 gives instruction-level fault granularity: each
    // runtime step executes exactly one PPU instruction.
    let mut rt = Runtime::new(mem, Budget::new(1), max_steps);
    rt.set_mode(RuntimeMode::FaultDriven);
    // Place HLE heap after TLS area (0x10400000 + 64KB for TLS).
    rt.set_hle_heap_base(0x10410000);
    rt.set_hle_nids(build_nid_map(&hle_bindings));
    rt.registry_mut().register_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = state;
        if let Some(pc) = dump_at_pc {
            unit.set_break_pc(pc, dump_skip);
        }
        unit
    });

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
    };
    let (outcome, boot_outcome) = step_loop(&mut rt, &mut loop_ctx);
    let t_loop = t_loop_start.elapsed();

    println!("outcome: {outcome}");
    println!("steps: {steps}");
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

    if let Some(path) = save_observation {
        save_boot_observation(path, &elf_data, rt.memory().as_bytes(), boot_outcome, steps);
    }
}

/// Build a boot-checkpoint observation and serialize it as JSON.
///
/// Region list is derived from the ELF's PT_LOAD segments: each segment
/// becomes one region named `seg{index}_{ro|rw}`. Writable segments
/// (.data/.bss) compare meaningfully only at matched boot states;
/// read-only segments (.text/.rodata) must byte-match at any checkpoint.
fn save_boot_observation(
    path: &str,
    elf_data: &[u8],
    final_memory: &[u8],
    outcome: cellgov_compare::BootOutcome,
    steps: usize,
) {
    let segments = match cellgov_ppu::loader::pt_load_segments(elf_data) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("save-observation: failed to enumerate PT_LOAD: {e:?}");
            return;
        }
    };
    let regions: Vec<cellgov_compare::RegionDescriptor> = segments
        .iter()
        .map(|s| {
            let kind = if s.writable { "rw" } else { "ro" };
            cellgov_compare::RegionDescriptor {
                name: format!("seg{}_{kind}", s.index),
                addr: s.vaddr,
                size: s.memsz,
            }
        })
        .collect();
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
                if *ctx.steps % 10_000 == 0 {
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
                let _ = rt.commit_step(&step.result);
                let t3 = Instant::now();

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
