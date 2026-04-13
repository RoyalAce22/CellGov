//! `run-game` subcommand: load a decrypted PS3 ELF and run the PPU
//! until fault, stall, or step limit.

use cellgov_core::{Runtime, StepError};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

use super::{die, load_file_or_die};

pub fn run_game(elf_path: &str, max_steps: usize, trace: bool) {
    let elf_data = load_file_or_die(elf_path);

    // Determine memory size from ELF segments.
    let required_size = cellgov_ppu::loader::required_memory_size(&elf_data)
        .unwrap_or_else(|e| die(&format!("failed to parse ELF: {e:?}")));

    // Round up to 64KB alignment and add headroom for stack/heap.
    let mem_size = ((required_size + 0xFFFF) & !0xFFFF) + 0x100000;
    let mut state = cellgov_ppu::state::PpuState::new();
    let mut mem = cellgov_mem::GuestMemory::new(mem_size);

    // Address 0 must stay zero: CRT0 linked lists use *NULL as a
    // termination sentinel. No exit stub is planted here.

    let load_result = cellgov_ppu::loader::load_ppu_elf(&elf_data, &mut mem, &mut state)
        .unwrap_or_else(|e| die(&format!("failed to load ELF: {e:?}")));

    // Parse and bind HLE import stubs.
    let hle_bindings = match cellgov_ppu::prx::parse_imports(&elf_data) {
        Ok(modules) => {
            let tramp_base = ((required_size + 0xFFF) & !0xFFF) as u32;
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
    println!("max_steps: {max_steps}");
    println!();

    // Budget=1 gives instruction-level fault granularity: each
    // runtime step executes exactly one PPU instruction.
    let mut rt = Runtime::new(mem, Budget::new(1), max_steps);
    rt.set_skip_hash_checkpoints(true);
    // Place HLE heap after TLS area (0x10400000 + 64KB for TLS).
    rt.set_hle_heap_base(0x10410000);
    // Build HLE NID table so the runtime can dispatch specific
    // HLE functions (TLS init, process exit, etc.).
    let hle_nid_map: std::collections::BTreeMap<u32, u32> =
        hle_bindings.iter().map(|b| (b.index, b.nid)).collect();
    rt.set_hle_nids(hle_nid_map);
    rt.registry_mut().register_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = state;
        unit
    });

    let mut steps: usize = 0;
    let mut hle_calls: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    let mut insn_coverage: std::collections::BTreeMap<String, usize> =
        std::collections::BTreeMap::new();

    let outcome = step_loop(
        &mut rt,
        &mut steps,
        &mut hle_calls,
        &mut insn_coverage,
        &hle_bindings,
        trace,
    );

    println!("outcome: {outcome}");
    println!("steps: {steps}");
    print_hle_summary(&hle_calls, &hle_bindings);
    print_insn_coverage(&insn_coverage);
}

fn step_loop(
    rt: &mut Runtime,
    steps: &mut usize,
    hle_calls: &mut std::collections::BTreeMap<u32, usize>,
    insn_coverage: &mut std::collections::BTreeMap<String, usize>,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
    trace: bool,
) -> String {
    loop {
        match rt.step() {
            Ok(step) => {
                *steps += 1;
                // Tally instruction coverage from the PC.
                if let Some(pc) = step.result.local_diagnostics.pc {
                    let m = rt.memory().as_bytes();
                    let a = pc as usize;
                    if a + 4 <= m.len() {
                        let raw = u32::from_be_bytes([m[a], m[a + 1], m[a + 2], m[a + 3]]);
                        let name = match cellgov_ppu::decode::decode(raw) {
                            Ok(insn) => format!("{insn:?}")
                                .split_once([' ', '{'])
                                .map(|(n, _)| n.to_string())
                                .unwrap_or_else(|| format!("{insn:?}")),
                            Err(_) => "DECODE_ERROR".to_string(),
                        };
                        *insn_coverage.entry(name).or_insert(0) += 1;
                    }
                }
                if trace {
                    print_trace_line(rt, &step.result, *steps, hle_bindings);
                }
                // Track HLE/LV2 calls before commit.
                if let Some(args) = &step.result.syscall_args {
                    if args[0] >= 0x10000 {
                        let idx = (args[0] - 0x10000) as u32;
                        *hle_calls.entry(idx).or_insert(0) += 1;
                    }
                }
                let _ = rt.commit_step(&step.result);

                if let Some(fault) = &step.result.fault {
                    break format_fault(rt, &step.result, fault, *steps);
                }
            }
            Err(StepError::NoRunnableUnit) => break format!("STALL after {} steps", steps),
            Err(StepError::MaxStepsExceeded) => break format!("MAX_STEPS after {} steps", steps),
            Err(StepError::TimeOverflow) => break format!("TIME_OVERFLOW after {} steps", steps),
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
        let m = rt.memory().as_bytes();
        let a = pc as usize;
        let raw = if a + 4 <= m.len() {
            u32::from_be_bytes([m[a], m[a + 1], m[a + 2], m[a + 3]])
        } else {
            0
        };
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
) -> String {
    let pc = result.local_diagnostics.pc;
    let pc_str = pc
        .map(|a| format!("0x{a:08x}"))
        .unwrap_or_else(|| "?".to_string());
    let desc = format!("{fault:?}");
    let detail = match fault {
        cellgov_effects::FaultKind::Guest(code) => {
            let fault_type = code & 0xFFFF_0000;
            match fault_type {
                0x0102_0000 => format!("PC_OUT_OF_RANGE at PC={pc_str}"),
                0x0105_0000 => {
                    let raw_str = pc
                        .and_then(|a| {
                            let a = a as usize;
                            let m = rt.memory().as_bytes();
                            if a + 4 <= m.len() {
                                let w = u32::from_be_bytes([m[a], m[a + 1], m[a + 2], m[a + 3]]);
                                Some(format!("0x{w:08x}"))
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "?".to_string());
                    format!("DECODE_ERROR at PC={pc_str} (raw={raw_str})")
                }
                0x0106_0000 => {
                    let ea_str = result
                        .local_diagnostics
                        .faulting_ea
                        .map(|a| format!("0x{a:08x}"))
                        .unwrap_or_else(|| "?".to_string());
                    format!("INVALID_ADDRESS at PC={pc_str} (ea={ea_str})")
                }
                0x0107_0000 => {
                    let nr = code & 0x0000_FFFF;
                    format!("UNSUPPORTED_SYSCALL (nr={nr}) at PC={pc_str}")
                }
                _ => format!("{desc} at PC={pc_str}"),
            }
        }
        _ => format!("{desc} at PC={pc_str}"),
    };
    format!("FAULT at step {steps}: {detail}")
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
    if !hle_calls.is_empty() {
        println!("hle_calls: {} distinct", hle_calls.len());
        for (idx, count) in hle_calls {
            let name = hle_bindings
                .get(*idx as usize)
                .map(|b| {
                    let func = cellgov_ppu::nid_db::lookup(b.nid)
                        .map(|(_, f)| f)
                        .unwrap_or("?");
                    format!("{}::{}", b.module, func)
                })
                .unwrap_or_else(|| format!("hle_{idx}"));
            println!("  {name}: {count}x");
        }
    }
}

fn print_insn_coverage(insn_coverage: &std::collections::BTreeMap<String, usize>) {
    if !insn_coverage.is_empty() {
        let mut sorted: Vec<_> = insn_coverage.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        println!("instruction_coverage: {} variants executed", sorted.len());
        for (name, count) in &sorted {
            println!("  {name}: {count}x");
        }
    }
}
