//! `run-game` subcommand: load a decrypted PS3 ELF and run the PPU
//! until fault, stall, or step limit.

use std::time::Instant;

use cellgov_core::{Runtime, RuntimeMode, StepError};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

use super::{die, load_file_or_die};

/// Fetch a 32-bit big-endian instruction word from guest memory at `pc`.
fn fetch_raw_at(rt: &Runtime, pc: u64) -> Option<u32> {
    let m = rt.memory().as_bytes();
    let a = pc as usize;
    if a + 4 <= m.len() {
        Some(u32::from_be_bytes([m[a], m[a + 1], m[a + 2], m[a + 3]]))
    } else {
        None
    }
}

pub fn run_game(
    elf_path: &str,
    max_steps: usize,
    trace: bool,
    profile: bool,
    firmware_dir: Option<&str>,
) {
    let t_start = Instant::now();
    let elf_data = load_file_or_die(elf_path);

    // Determine memory size from ELF segments.
    let required_size = cellgov_ppu::loader::required_memory_size(&elf_data)
        .unwrap_or_else(|e| die(&format!("failed to parse ELF: {e:?}")));

    // Round up to 64KB alignment. Guest memory must be large enough for
    // the game, PRX modules, and kernel memory allocations (at 0x30000000+).
    let min_for_kernel = 0x3100_0000usize; // covers kernel alloc region
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

    // Patch the game's libc malloc entry points to redirect to the
    // _sys_malloc HLE stub, bypassing uninitialized heap arenas.
    // Deliberately bounded to flOw's known malloc offsets for initial testing.
    patch_malloc(&hle_bindings, &mut mem);

    // Set up stack
    let stack_top = (mem_size as u64) - 0x1000;
    state.gpr[1] = stack_top;
    state.lr = 0;

    println!("elf: {elf_path}");
    println!("memory: {} MB", mem_size / (1024 * 1024));
    println!(
        "entry: 0x{:x} (OPD) -> pc=0x{:x} toc=0x{:x}",
        load_result.entry, state.pc, state.gpr[2]
    );
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
    };
    let outcome = step_loop(&mut rt, &mut loop_ctx);
    let t_loop = t_loop_start.elapsed();

    println!("outcome: {outcome}");
    println!("steps: {steps}");
    print_hle_summary(&hle_calls, &hle_bindings);
    print_insn_coverage(&insn_coverage);

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

const PC_RING_SIZE: usize = 16;

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
}

fn step_loop(rt: &mut Runtime, ctx: &mut StepLoopCtx<'_>) -> String {
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
                // Track HLE/LV2 calls before commit.
                if let Some(args) = &step.result.syscall_args {
                    if args[0] >= 0x10000 {
                        let idx = (args[0] - 0x10000) as u32;
                        *ctx.hle_calls.entry(idx).or_insert(0) += 1;
                    }
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
                    break format_fault(
                        rt,
                        &step.result,
                        fault,
                        *ctx.steps,
                        &ctx.pc_ring,
                        ctx.pc_ring_pos,
                    );
                }
            }
            Err(StepError::NoRunnableUnit) => {
                break format!("STALL after {} steps", ctx.steps);
            }
            Err(StepError::MaxStepsExceeded) => {
                break format!("MAX_STEPS after {} steps", ctx.steps);
            }
            Err(StepError::TimeOverflow) => {
                break format!("TIME_OVERFLOW after {} steps", ctx.steps);
            }
        }
    }
}

fn print_trace_line(
    rt: &Runtime,
    result: &cellgov_exec::ExecutionStepResult,
    steps: usize,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
) {
    if let Some(pc) = result.local_diagnostics.pc {
        let raw = fetch_raw_at(rt, pc).unwrap_or(0);
        println!(
            "[{steps:>4}] PC=0x{pc:08x}  raw=0x{raw:08x}  yr={:?}",
            result.yield_reason
        );
    }
    if let Some(args) = &result.syscall_args {
        if args[0] >= 0x10000 {
            let idx = (args[0] - 0x10000) as u32;
            let name = hle_bindings
                .get(idx as usize)
                .map(|b| {
                    let func = cellgov_ppu::nid_db::lookup(b.nid)
                        .map(|(_, f)| f)
                        .unwrap_or("?");
                    format!("{}::{}", b.module, func)
                })
                .unwrap_or_else(|| format!("hle_{idx}"));
            println!("       -> HLE #{idx}: {name}");
        } else if args[0] == 403 {
            let buf = args[2] as usize;
            let len = args[3] as usize;
            let m = rt.memory().as_bytes();
            if buf + len <= m.len() {
                let text = String::from_utf8_lossy(&m[buf..buf + len]);
                print!("       -> tty: {text}");
                if !text.ends_with('\n') {
                    println!();
                }
            } else {
                println!("       -> LV2 tty_write (oob)");
            }
        } else {
            println!("       -> LV2 syscall {}", args[0]);
        }
    }
}

fn format_fault(
    rt: &Runtime,
    result: &cellgov_exec::ExecutionStepResult,
    fault: &cellgov_effects::FaultKind,
    steps: usize,
    pc_ring: &[u64; PC_RING_SIZE],
    pc_ring_pos: usize,
) -> String {
    let pc = result.local_diagnostics.pc;
    let pc_str = pc
        .map(|a| format!("0x{a:08x}"))
        .unwrap_or_else(|| "?".to_string());
    use cellgov_ppu::{
        FAULT_DECODE_ERROR, FAULT_INVALID_ADDRESS, FAULT_PC_OUT_OF_RANGE, FAULT_UNSUPPORTED_SYSCALL,
    };
    let detail = match fault {
        cellgov_effects::FaultKind::Guest(code) => {
            let fault_type = code & 0xFFFF_0000;
            match fault_type {
                FAULT_PC_OUT_OF_RANGE => format!("PC_OUT_OF_RANGE at PC={pc_str}"),
                FAULT_DECODE_ERROR => {
                    let raw_str = pc
                        .and_then(|a| fetch_raw_at(rt, a))
                        .map(|w| format!("0x{w:08x}"))
                        .unwrap_or_else(|| "?".to_string());
                    format!("DECODE_ERROR at PC={pc_str} (raw={raw_str})")
                }
                FAULT_INVALID_ADDRESS => {
                    let ea_str = result
                        .local_diagnostics
                        .faulting_ea
                        .map(|a| format!("0x{a:08x}"))
                        .unwrap_or_else(|| "?".to_string());
                    format!("INVALID_ADDRESS at PC={pc_str} (ea={ea_str})")
                }
                FAULT_UNSUPPORTED_SYSCALL => {
                    let nr = code & 0x0000_FFFF;
                    format!("UNSUPPORTED_SYSCALL (nr={nr}) at PC={pc_str}")
                }
                _ => format!("Guest(0x{code:08x}) at PC={pc_str}"),
            }
        }
        _ => format!("Validation at PC={pc_str}"),
    };
    let mut out = format!("FAULT at step {steps}: {detail}");

    // Register dump if available.
    if let Some(regs) = &result.local_diagnostics.fault_regs {
        out.push_str("\n  registers:");
        for (i, &val) in regs.gprs.iter().enumerate() {
            if i % 4 == 0 {
                out.push_str("\n    ");
            }
            out.push_str(&format!("r{i:<2}=0x{val:016x}  "));
        }
        out.push_str(&format!(
            "\n    LR=0x{:016x}  CTR=0x{:016x}  CR=0x{:08x}",
            regs.lr, regs.ctr, regs.cr
        ));
    }

    // Mini-trace: last N PCs from the ring buffer.
    let filled = pc_ring_pos.min(PC_RING_SIZE);
    if filled > 0 {
        out.push_str(&format!("\n  last {filled} PCs:"));
        let start = pc_ring_pos.saturating_sub(PC_RING_SIZE);
        for i in start..pc_ring_pos {
            let pc = pc_ring[i % PC_RING_SIZE];
            out.push_str(&format!("\n    0x{pc:08x}"));
        }
    }

    out
}

/// Execute a PRX module's module_start function through the PPU
/// interpreter. Takes ownership of guest memory, runs until the function
/// returns (PC reaches the LR sentinel at 0) or a fault/stall occurs,
/// then returns the modified memory.
fn run_module_start(
    mem: cellgov_mem::GuestMemory,
    prx_info: &PrxLoadInfo,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
    max_steps: usize,
) -> cellgov_mem::GuestMemory {
    let ms = match prx_info.module_start {
        Some(opd) => opd,
        None => {
            println!(
                "module_start: {} has no module_start, skipping",
                prx_info.name
            );
            return mem;
        }
    };

    println!(
        "module_start: {} at pc=0x{:x} toc=0x{:x}",
        prx_info.name, ms.code, ms.toc,
    );

    // Set up PPU state for module_start.
    let mut ms_state = cellgov_ppu::state::PpuState::new();
    ms_state.pc = ms.code;
    ms_state.gpr[2] = ms.toc;
    // Stack below game's stack area.
    let mem_size = mem.size();
    ms_state.gpr[1] = mem_size - 0x2000;
    // TLS base: PPC64 convention is r13 = TLS_area + 0x7030.
    ms_state.gpr[13] = TLS_BASE + 0x30 + 0x7000;
    // LR = 0: when module_start does blr, PC becomes 0. Address 0 is
    // all-zeros which fails to decode, producing a fault that signals
    // "module_start returned."
    ms_state.lr = 0;

    let mut rt = Runtime::new(mem, Budget::new(1), max_steps);
    rt.set_mode(RuntimeMode::FaultDriven);
    rt.set_hle_heap_base(0x10410000);
    rt.set_hle_nids(build_nid_map(hle_bindings));
    rt.registry_mut().register_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = ms_state;
        unit
    });

    // Run module_start with a simplified step loop.
    let t_start = Instant::now();
    let mut steps: usize = 0;
    let mut distinct_pcs = std::collections::BTreeSet::new();

    let outcome: String = loop {
        match rt.step() {
            Ok(step) => {
                steps += 1;

                if let Some(pc) = step.result.local_diagnostics.pc {
                    distinct_pcs.insert(pc);
                }

                // Capture TTY output from module_start.
                if let Some(args) = &step.result.syscall_args {
                    if args[0] == 403 {
                        let buf = args[2] as usize;
                        let len = (args[3] as usize).min(256);
                        let m = rt.memory().as_bytes();
                        if buf + len <= m.len() {
                            let text = String::from_utf8_lossy(&m[buf..buf + len]);
                            print!("  module_start TTY: {text}");
                        }
                    }
                }

                // Progress checkpoint every 10K steps.
                if steps % 10_000 == 0 {
                    println!(
                        "  module_start [{:>6}] {} distinct PCs",
                        steps,
                        distinct_pcs.len(),
                    );
                }

                let _ = rt.commit_step(&step.result);

                if let Some(fault) = &step.result.fault {
                    let fault_pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    let guest_code = match fault {
                        cellgov_effects::FaultKind::Guest(c) => Some(*c),
                        _ => None,
                    };

                    if fault_pc == 0
                        && guest_code
                            .is_some_and(|c| (c & 0xFFFF_0000) == cellgov_ppu::FAULT_DECODE_ERROR)
                    {
                        // PC = 0 after blr: module_start returned normally.
                        break format!("RETURNED after {} steps", steps);
                    }
                    let code_str = guest_code
                        .map(|c| format!("0x{c:08x}"))
                        .unwrap_or_else(|| format!("{fault:?}"));
                    // Print register dump if available.
                    if let Some(ref regs) = step.result.local_diagnostics.fault_regs {
                        println!("  module_start fault registers:");
                        for row in 0..8 {
                            let base = row * 4;
                            println!(
                                "    r{:<2}=0x{:016x}  r{:<2}=0x{:016x}  r{:<2}=0x{:016x}  r{:<2}=0x{:016x}",
                                base, regs.gprs[base],
                                base+1, regs.gprs[base+1],
                                base+2, regs.gprs[base+2],
                                base+3, regs.gprs[base+3],
                            );
                        }
                        println!(
                            "    LR=0x{:016x}  CTR=0x{:016x}  CR=0x{:08x}",
                            regs.lr, regs.ctr, regs.cr,
                        );
                    }
                    break format!(
                        "FAULT {} at pc=0x{:x} after {} steps",
                        code_str, fault_pc, steps,
                    );
                }
            }
            Err(StepError::NoRunnableUnit) => {
                break format!("STALL after {} steps", steps);
            }
            Err(StepError::MaxStepsExceeded) => {
                break format!("MAX_STEPS after {} steps", steps);
            }
            Err(e) => {
                break format!("{e:?} after {steps} steps");
            }
        }
    };

    let elapsed = t_start.elapsed();
    println!(
        "module_start: {} -- {} steps, {} distinct PCs, {:.1?}",
        outcome,
        steps,
        distinct_pcs.len(),
        elapsed,
    );

    rt.into_memory()
}

/// Pre-initialize the TLS area from the game ELF's PT_TLS segment.
///
/// On real PS3, the kernel does this during process creation, before
/// any module_start runs. This copies the TLS template to the TLS base
/// address (0x10400000 + 0x30) and zeros the BSS portion.
fn build_nid_map(
    bindings: &[cellgov_ppu::prx::HleBinding],
) -> std::collections::BTreeMap<u32, u32> {
    bindings.iter().map(|b| (b.index, b.nid)).collect()
}

/// TLS base address in guest memory. Matches the HLE sys_initialize_tls
/// allocation in `cellgov_core::hle`.
const TLS_BASE: u64 = 0x10400000;

fn pre_init_tls(elf_data: &[u8], mem: &mut cellgov_mem::GuestMemory) {
    let tls = match cellgov_ppu::loader::find_tls_segment(elf_data) {
        Some(t) => t,
        None => return,
    };

    let tls_data_start = TLS_BASE as usize + 0x30;
    let p_vaddr = tls.vaddr as usize;
    let p_filesz = tls.filesz as usize;
    let p_memsz = tls.memsz as usize;

    // Copy TLS initialization image from the ELF's loaded memory.
    let m = mem.as_bytes();
    if p_vaddr + p_filesz <= m.len() && tls_data_start + p_filesz <= m.len() {
        let init_data: Vec<u8> = m[p_vaddr..p_vaddr + p_filesz].to_vec();
        if let Some(range) = cellgov_mem::ByteRange::new(
            cellgov_mem::GuestAddr::new(tls_data_start as u64),
            p_filesz as u64,
        ) {
            let _ = mem.apply_commit(range, &init_data);
        }
    }

    // Zero BSS portion.
    let bss_start = tls_data_start + p_filesz;
    let bss_len = p_memsz.saturating_sub(p_filesz);
    if bss_len > 0 && bss_start + bss_len <= mem.as_bytes().len() {
        let zeros = vec![0u8; bss_len];
        if let Some(range) = cellgov_mem::ByteRange::new(
            cellgov_mem::GuestAddr::new(bss_start as u64),
            bss_len as u64,
        ) {
            let _ = mem.apply_commit(range, &zeros);
        }
    }

    println!(
        "tls: pre-initialized from PT_TLS at 0x{:x} (filesz=0x{:x}, memsz=0x{:x}) -> 0x{:x}",
        p_vaddr, p_filesz, p_memsz, TLS_BASE
    );
}

/// Summary of a loaded firmware PRX module.
struct PrxLoadInfo {
    name: String,
    base: u64,
    toc: u64,
    relocs_applied: usize,
    resolved: usize,
    total_imports: usize,
    /// module_start entry point (code, toc) if present.
    module_start: Option<cellgov_ppu::sprx::LoadedOpd>,
}

/// Attempt to load liblv2.prx from the firmware directory and resolve
/// game imports against its exports. For each imported NID that the
/// module exports, the GOT entry is re-patched to point to the real
/// OPD instead of the HLE trampoline.
fn load_firmware_prx(
    firmware_dir: Option<&str>,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
    mem: &mut cellgov_mem::GuestMemory,
    tramp_base: u32,
) -> Option<PrxLoadInfo> {
    let dir = firmware_dir?;
    let prx_path = std::path::PathBuf::from(dir).join("liblv2.prx");
    if !prx_path.exists() {
        println!("prx: {}: not found, using pure HLE", prx_path.display());
        return None;
    }

    let prx_data = match std::fs::read(&prx_path) {
        Ok(d) => d,
        Err(e) => {
            println!("prx: failed to read {}: {e}", prx_path.display());
            return None;
        }
    };

    let parsed = match cellgov_ppu::sprx::parse_prx(&prx_data) {
        Ok(p) => p,
        Err(e) => {
            println!("prx: failed to parse liblv2.prx: {e:?}");
            return None;
        }
    };

    // Place the PRX above the game segments and trampoline area.
    // Trampolines: tramp_base .. tramp_base + bindings*24. Round up to 64KB.
    let tramp_end = tramp_base as u64 + (hle_bindings.len() as u64) * 24;
    let prx_base = (tramp_end + 0xFFFF) & !0xFFFF;

    let loaded = match cellgov_ppu::sprx::load_prx(&parsed, mem, prx_base) {
        Ok(l) => l,
        Err(e) => {
            println!("prx: failed to load liblv2.prx at 0x{prx_base:x}: {e:?}");
            return None;
        }
    };

    // NIDs with working HLE implementations that should NOT be replaced
    // with real PRX code. These are kept as HLE trampolines because the
    // real implementations depend on full module_start initialization
    // (TLS, heap arenas) which may not complete.
    const HLE_KEEP_NIDS: &[u32] = &[
        0x744680a2, // sys_initialize_tls
        0xebe5f72f, // _sys_malloc
        0xfc52a7a9, // _sys_free
        0x1573dc3f, // _sys_memset
        0xe6f2c1e7, // sys_process_exit
        0xaff080a4, // _sys_heap_create_heap
        0x2f85c0ef, // sys_lwmutex_create
        0xc3476d0c, // sys_lwmutex_lock
        0x1bc200f4, // sys_lwmutex_unlock
        0xaeb78725, // sys_lwmutex_trylock
        0x8461e528, // sys_time_get_system_time
        0x350d454e, // sys_ppu_thread_get_id
        0x24a1ea07, // sys_ppu_thread_create
        0x4f7172c9, // sys_process_is_stack
        0xa2c7ba64, // sys_prx_exitspawn_with_level
    ];

    // Re-patch GOT entries for NIDs that the loaded module exports,
    // unless the NID is in the HLE keep list.
    let mut resolved = 0;
    let mut kept_hle = 0;
    for binding in hle_bindings {
        if HLE_KEEP_NIDS.contains(&binding.nid) {
            kept_hle += 1;
            continue;
        }
        if let Some(&real_opd_addr) = loaded.exports.get(&binding.nid) {
            let opd_addr_u32 = real_opd_addr as u32;
            let got_range = cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new(binding.stub_addr as u64),
                4,
            );
            if let Some(range) = got_range {
                let _ = mem.apply_commit(range, &opd_addr_u32.to_be_bytes());
            }
            resolved += 1;
        }
    }

    println!(
        "prx: loaded {} -- {} exports, {}/{} resolved to real code, {} kept as HLE",
        loaded.name,
        loaded.exports.len(),
        resolved,
        hle_bindings.len(),
        kept_hle,
    );

    Some(PrxLoadInfo {
        name: loaded.name,
        base: loaded.base,
        toc: loaded.toc,
        relocs_applied: loaded.relocs_applied,
        resolved,
        total_imports: hle_bindings.len(),
        module_start: loaded.module_start,
    })
}

fn patch_malloc(hle_bindings: &[cellgov_ppu::prx::HleBinding], mem: &mut cellgov_mem::GuestMemory) {
    const NID_SYS_MALLOC: u32 = 0xebe5f72f;
    if let Some(b) = hle_bindings.iter().find(|b| b.nid == NID_SYS_MALLOC) {
        let syscall_nr = 0x10000_u32 + b.index;
        let hi = (syscall_nr >> 16) & 0xFFFF;
        let lo = syscall_nr & 0xFFFF;
        let lis: u32 = (15 << 26) | (11 << 21) | hi;
        let ori: u32 = (24 << 26) | (11 << 21) | (11 << 16) | lo;
        let sc: u32 = 0x4400_0002;
        let blr: u32 = 0x4E80_0020;
        let mut code = [0u8; 16];
        code[0..4].copy_from_slice(&lis.to_be_bytes());
        code[4..8].copy_from_slice(&ori.to_be_bytes());
        code[8..12].copy_from_slice(&sc.to_be_bytes());
        code[12..16].copy_from_slice(&blr.to_be_bytes());
        for addr in [0x6b738c_u64, 0x6b54fc, 0x6c2c54] {
            if let Some(r) = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 16) {
                let _ = mem.apply_commit(r, &code);
            }
        }
    }
}

fn print_hle_summary(
    hle_calls: &std::collections::BTreeMap<u32, usize>,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
) {
    let called_count = hle_calls.len();
    let total_count = hle_bindings.len();
    let uncalled_count = total_count - called_count.min(total_count);
    println!("hle_imports: {total_count} bound, {called_count} called, {uncalled_count} uncalled");

    if !hle_calls.is_empty() {
        println!("  called:");
        for (idx, count) in hle_calls {
            let (name, class) = hle_bindings
                .get(*idx as usize)
                .map(|b| {
                    let func = cellgov_ppu::nid_db::lookup(b.nid)
                        .map(|(_, f)| f)
                        .unwrap_or("?");
                    (
                        format!("{}::{}", b.module, func),
                        cellgov_ppu::nid_db::stub_classification(b.nid),
                    )
                })
                .unwrap_or_else(|| (format!("hle_{idx}"), "?"));
            println!("    {name}: {count}x [{class}]");
        }
    }

    // Show uncalled imports grouped by classification.
    let uncalled: Vec<_> = hle_bindings
        .iter()
        .filter(|b| !hle_calls.contains_key(&b.index))
        .collect();
    if !uncalled.is_empty() {
        let stateful: Vec<_> = uncalled
            .iter()
            .filter(|b| cellgov_ppu::nid_db::stub_classification(b.nid) != "noop-safe")
            .collect();
        if !stateful.is_empty() {
            println!("  uncalled (non-noop):");
            for b in &stateful {
                let func = cellgov_ppu::nid_db::lookup(b.nid)
                    .map(|(_, f)| f)
                    .unwrap_or("?");
                let class = cellgov_ppu::nid_db::stub_classification(b.nid);
                println!("    {}::{} [{class}]", b.module, func);
            }
        }
        let noop_count = uncalled.len() - stateful.len();
        if noop_count > 0 {
            println!("  uncalled (noop-safe): {noop_count} functions");
        }
    }
}

fn print_insn_coverage(insn_coverage: &std::collections::BTreeMap<&'static str, usize>) {
    if !insn_coverage.is_empty() {
        let mut sorted: Vec<_> = insn_coverage.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        println!("instruction_coverage: {} variants executed", sorted.len());
        for (name, count) in &sorted {
            println!("  {name}: {count}x");
        }
    }
}
