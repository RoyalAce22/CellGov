//! Firmware PRX loading, module_start execution, and TLS
//! pre-initialization for `run-game`.
//!
//! Split out of `game.rs` to keep the core boot driver manageable.

use std::time::Instant;

use cellgov_core::{Runtime, RuntimeMode, StepError};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

use super::diag::fetch_raw_at;

/// Summary of a loaded firmware PRX module.
pub(super) struct PrxLoadInfo {
    pub(super) name: String,
    pub(super) base: u64,
    pub(super) toc: u64,
    pub(super) relocs_applied: usize,
    pub(super) resolved: usize,
    pub(super) total_imports: usize,
    /// module_start entry point (code, toc) if present.
    pub(super) module_start: Option<cellgov_ppu::sprx::LoadedOpd>,
}

/// Pre-initialize the TLS area from the game ELF's PT_TLS segment.
///
/// On real PS3, the kernel does this during process creation, before
/// any module_start runs. This copies the TLS template to the TLS base
/// address (0x10400000 + 0x30) and zeros the BSS portion.
pub(super) fn build_nid_map(
    bindings: &[cellgov_ppu::prx::HleBinding],
) -> std::collections::BTreeMap<u32, u32> {
    bindings.iter().map(|b| (b.index, b.nid)).collect()
}

/// TLS base address in guest memory. Matches the HLE sys_initialize_tls
/// allocation in `cellgov_core::hle`.
pub(super) const TLS_BASE: u64 = 0x10400000;

pub(super) fn pre_init_tls(elf_data: &[u8], mem: &mut cellgov_mem::GuestMemory) {
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

/// Execute a PRX module's module_start function through the PPU
/// interpreter. Takes ownership of guest memory, runs until the function
/// returns (PC reaches the LR sentinel at 0) or a fault/stall occurs,
/// then returns the modified memory.
pub(super) fn run_module_start(
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
    let mut hle_calls: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    let mut lv2_calls: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
    // PC-hit histogram: if module_start enters a busy loop, the loop body's
    // PCs dominate the top entries because they are hit tens of thousands of
    // times while initialization PCs are hit once or a few times.
    let mut pc_hits: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
    // Ring buffer of the last MS_PC_RING_SIZE PCs. On fault, this shows
    // the exact execution sequence leading up to the fault, which is how
    // we identify which instruction set a register to a bad value.
    const MS_PC_RING_SIZE: usize = 32;
    let mut pc_ring: [u64; MS_PC_RING_SIZE] = [0; MS_PC_RING_SIZE];
    let mut pc_ring_pos: usize = 0;
    // Ring of recent syscalls (pc at sc, syscall number, r3, r4, r5).
    // On a fault that occurs post-syscall, this identifies which
    // syscall's return value the code was processing when it faulted.
    const MS_SC_RING_SIZE: usize = 8;
    let mut sc_ring: [(u64, u64, u64, u64, u64); MS_SC_RING_SIZE] =
        [(0, 0, 0, 0, 0); MS_SC_RING_SIZE];
    let mut sc_ring_pos: usize = 0;

    let outcome: String = loop {
        match rt.step() {
            Ok(step) => {
                steps += 1;

                if let Some(pc) = step.result.local_diagnostics.pc {
                    distinct_pcs.insert(pc);
                    *pc_hits.entry(pc).or_insert(0) += 1;
                    pc_ring[pc_ring_pos % MS_PC_RING_SIZE] = pc;
                    pc_ring_pos += 1;
                }

                // Track HLE/LV2 calls so a busy loop reveals what it polls.
                if let Some(args) = &step.result.syscall_args {
                    if args[0] >= 0x10000 {
                        let idx = (args[0] - 0x10000) as u32;
                        *hle_calls.entry(idx).or_insert(0) += 1;
                    } else {
                        *lv2_calls.entry(args[0]).or_insert(0) += 1;
                    }
                    let sc_pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    sc_ring[sc_ring_pos % MS_SC_RING_SIZE] =
                        (sc_pc, args[0], args[1], args[2], args[3]);
                    sc_ring_pos += 1;

                    // Capture TTY output from module_start.
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
                    let raw_str = fetch_raw_at(&rt, fault_pc)
                        .map(|w| format!("0x{w:08x}"))
                        .unwrap_or_else(|| "?".to_string());
                    // Pre-fault syscall ring: identifies the most recent
                    // syscall numbers and their r3/r4/r5 arguments. When
                    // the fault is in post-syscall handling code, this
                    // tells us which syscall's return value or output
                    // pointer is being processed.
                    let sc_entries = sc_ring_pos.min(MS_SC_RING_SIZE);
                    if sc_entries > 0 {
                        println!("  module_start last {} syscalls before fault:", sc_entries);
                        for i in 0..sc_entries {
                            let idx =
                                (sc_ring_pos + MS_SC_RING_SIZE - sc_entries + i) % MS_SC_RING_SIZE;
                            let (pc, num, a1, a2, a3) = sc_ring[idx];
                            let kind = if num >= 0x10000 {
                                format!("HLE#{}", num - 0x10000)
                            } else {
                                format!("LV2 {}", num)
                            };
                            println!(
                                "    [{:>2}] pc=0x{:08x} {}  r3=0x{:x} r4=0x{:x} r5=0x{:x}",
                                (i as i64) - (sc_entries as i64 - 1),
                                pc,
                                kind,
                                a1,
                                a2,
                                a3,
                            );
                        }
                    }
                    // Pre-fault PC ring: oldest-first. Useful when the
                    // fault is a memory access using a register that was
                    // set wrong a few instructions earlier -- we can walk
                    // back to the exact setter.
                    let entries = pc_ring_pos.min(MS_PC_RING_SIZE);
                    if entries > 0 {
                        println!("  module_start last {} PCs before fault:", entries);
                        for i in 0..entries {
                            let idx =
                                (pc_ring_pos + MS_PC_RING_SIZE - entries + i) % MS_PC_RING_SIZE;
                            let pc = pc_ring[idx];
                            let raw = fetch_raw_at(&rt, pc)
                                .map(|w| format!("0x{w:08x}"))
                                .unwrap_or_else(|| "?".into());
                            let name = fetch_raw_at(&rt, pc)
                                .and_then(|w| cellgov_ppu::decode::decode(w).ok())
                                .map(|insn| insn.variant_name().to_string())
                                .unwrap_or_else(|| "?".into());
                            println!(
                                "    [{:>2}] pc=0x{:08x} raw={} {}",
                                (i as i64) - (entries as i64 - 1),
                                pc,
                                raw,
                                name,
                            );
                        }
                    }
                    break format!(
                        "FAULT {code_str} at pc=0x{fault_pc:x} (raw={raw_str}) after {steps} steps"
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
    if !hle_calls.is_empty() {
        println!("  module_start HLE calls:");
        let mut sorted: Vec<_> = hle_calls.iter().collect();
        sorted.sort_by_key(|&(_, c)| std::cmp::Reverse(*c));
        for (idx, count) in sorted.iter().take(10) {
            let name = hle_bindings
                .get(**idx as usize)
                .and_then(|b| cellgov_ppu::nid_db::lookup(b.nid).map(|(m, f)| format!("{m}::{f}")))
                .unwrap_or_else(|| format!("hle_{idx}"));
            println!("    {count:>8}x  {name}");
        }
    }
    if !lv2_calls.is_empty() {
        println!("  module_start LV2 syscalls:");
        let mut sorted: Vec<_> = lv2_calls.iter().collect();
        sorted.sort_by_key(|&(_, c)| std::cmp::Reverse(*c));
        for (num, count) in sorted.iter().take(10) {
            println!("    {count:>8}x  syscall {num}");
        }
    }

    // Top-20 PC hit counts. With a pure-PPU busy loop the hottest PCs
    // are the loop body; the raw word at each PC lets us read back the
    // exact instruction in the cycle.
    if !pc_hits.is_empty() {
        println!("  module_start top PCs by hit count:");
        let mut sorted: Vec<_> = pc_hits.iter().collect();
        sorted.sort_by_key(|&(_, c)| std::cmp::Reverse(*c));
        for (pc, count) in sorted.iter().take(20) {
            let raw = fetch_raw_at(&rt, **pc)
                .map(|w| format!("0x{w:08x}"))
                .unwrap_or_else(|| "?".to_string());
            let disasm = fetch_raw_at(&rt, **pc)
                .and_then(|w| cellgov_ppu::decode::decode(w).ok())
                .map(|insn| insn.variant_name().to_string())
                .unwrap_or_else(|| "?".into());
            println!("    {count:>10}x  PC=0x{:08x}  raw={raw}  {disasm}", **pc);
        }
    }

    rt.into_memory()
}

/// Attempt to load liblv2.prx from the PS3 firmware directory and
/// resolve game imports against its exports. The firmware directory
/// is expected to contain decrypted PS3 system PRX modules; CellGov
/// does not ship them, and the files are not RPCS3-specific.
///
/// For each imported NID that the module exports, the GOT entry is
/// re-patched to point to the real OPD instead of the HLE trampoline.
pub(super) fn load_firmware_prx(
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
        0xbdb18f83, // _sys_malloc
        0xf7f7fb20, // _sys_free
        0x68b9b011, // _sys_memset
        0xe6f2c1e7, // sys_process_exit
        0xb2fcf2c8, // _sys_heap_create_heap
        0x2f85c0ef, // sys_lwmutex_create
        0x1573dc3f, // sys_lwmutex_lock
        0xc3476d0c, // sys_lwmutex_destroy
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
