//! Firmware PRX loading, module_start execution, and TLS pre-init
//! for `run-game`.

use std::time::Instant;

use cellgov_core::{Runtime, RuntimeMode, StepError};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

use super::diag::fetch_raw_at;
use crate::cli::exit::die;

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

/// Build an HLE-index to NID map for fast runtime lookup.
///
/// Duplicate indices indicate an upstream bug in the binder; the
/// debug_assert surfaces it instead of letting the later binding
/// silently overwrite the earlier one.
pub(super) fn build_nid_map(
    bindings: &[cellgov_ppu::prx::HleBinding],
) -> std::collections::BTreeMap<u32, u32> {
    let mut map = std::collections::BTreeMap::new();
    for b in bindings {
        if let Some(prev) = map.insert(b.index, b.nid) {
            debug_assert!(
                false,
                "duplicate HleBinding index {idx}: previous nid 0x{prev:08x} replaced by 0x{nid:08x}",
                idx = b.index,
                nid = b.nid
            );
            eprintln!(
                "warning: duplicate HleBinding index {idx}: nid 0x{prev:08x} replaced by 0x{nid:08x}",
                idx = b.index,
                nid = b.nid
            );
        }
    }
    map
}

/// TLS base in guest memory. Matches the HLE sys_initialize_tls
/// allocation in `cellgov_core::hle`.
pub(super) const TLS_BASE: u64 = 0x10400000;

/// Pre-initialize TLS from the ELF's PT_TLS segment.
///
/// Copies the TLS template to `TLS_BASE + 0x30` and zeros the BSS
/// portion. On real PS3 the kernel does this during process creation
/// before any module_start runs.
pub(super) fn pre_init_tls(elf_data: &[u8], mem: &mut cellgov_mem::GuestMemory) {
    let tls = match cellgov_ppu::loader::find_tls_segment(elf_data) {
        Some(t) => t,
        None => return,
    };

    let tls_data_start = TLS_BASE as usize + 0x30;
    let p_vaddr = tls.vaddr as usize;
    let p_filesz = tls.filesz as usize;
    let p_memsz = tls.memsz as usize;

    // Copy the TLS template; a skipped copy here means r13-relative
    // reads in module_start will see uninitialized TLS bytes.
    let mut copy_ok = true;
    let m = mem.as_bytes();
    // checked_add: malformed PT_TLS fields must not wrap on 32-bit
    // usize hosts and falsely pass the bounds check.
    let src_end = p_vaddr.checked_add(p_filesz);
    let dst_end = tls_data_start.checked_add(p_filesz);
    if src_end.is_none_or(|e| e > m.len()) || dst_end.is_none_or(|e| e > m.len()) {
        eprintln!(
            "tls: skipping template copy: src=0x{:x}+0x{:x} or dst=0x{:x}+0x{:x} exceeds guest memory ({} bytes)",
            p_vaddr,
            p_filesz,
            tls_data_start,
            p_filesz,
            m.len()
        );
        copy_ok = false;
    } else {
        let init_data: Vec<u8> = m[p_vaddr..p_vaddr + p_filesz].to_vec();
        match cellgov_mem::ByteRange::new(
            cellgov_mem::GuestAddr::new(tls_data_start as u64),
            p_filesz as u64,
        ) {
            None => {
                eprintln!("tls: template copy: invalid byte range at 0x{tls_data_start:x}");
                copy_ok = false;
            }
            Some(range) => {
                if let Err(e) = mem.apply_commit(range, &init_data) {
                    eprintln!(
                        "tls: template copy to 0x{tls_data_start:x} FAILED ({e:?}); TLS not initialized"
                    );
                    copy_ok = false;
                }
            }
        }
    }

    let bss_start = tls_data_start + p_filesz;
    let bss_len = p_memsz.saturating_sub(p_filesz);
    let mut bss_ok = true;
    if bss_len > 0 {
        let bss_end = bss_start.checked_add(bss_len);
        if bss_end.is_none_or(|e| e > mem.as_bytes().len()) {
            eprintln!("tls: skipping BSS zero: 0x{bss_start:x}+0x{bss_len:x} exceeds guest memory");
            bss_ok = false;
        } else {
            let zeros = vec![0u8; bss_len];
            match cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new(bss_start as u64),
                bss_len as u64,
            ) {
                None => {
                    eprintln!("tls: BSS zero: invalid byte range at 0x{bss_start:x}");
                    bss_ok = false;
                }
                Some(range) => {
                    if let Err(e) = mem.apply_commit(range, &zeros) {
                        eprintln!(
                            "tls: BSS zero at 0x{bss_start:x} FAILED ({e:?}); BSS contains stale bytes"
                        );
                        bss_ok = false;
                    }
                }
            }
        }
    }

    if copy_ok && bss_ok {
        println!(
            "tls: pre-initialized from PT_TLS at 0x{:x} (filesz=0x{:x}, memsz=0x{:x}) -> 0x{:x}",
            p_vaddr, p_filesz, p_memsz, TLS_BASE
        );
    } else {
        eprintln!(
            "tls: pre-initialization INCOMPLETE (template_copy={copy_ok}, bss_zero={bss_ok}); guest TLS reads will see unexpected bytes"
        );
    }
}

/// Run a PRX module's module_start to completion or fault.
///
/// Takes ownership of guest memory, runs until the function returns
/// (PC reaches the LR=0 sentinel) or a fault/stall occurs, then
/// returns the modified memory. A decode-error fault at PC=0 with
/// LR=0 at fault time is treated as a clean return.
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

    let mut ms_state = cellgov_ppu::state::PpuState::new();
    ms_state.pc = ms.code;
    ms_state.gpr[2] = ms.toc;
    // Stack offset below the game's stack_top so the two do not
    // clobber each other if they ever coexist (LV2 runs module_start
    // to completion first, so in practice they do not).
    ms_state.gpr[1] = super::PS3_PRIMARY_STACK_BASE + 0x8000;
    // PPC64 convention: r13 = TLS_area + 0x7030.
    ms_state.gpr[13] = TLS_BASE + 0x30 + 0x7000;
    // LR=0 sentinel: blr from module_start jumps to PC=0, where the
    // all-zero word fails to decode and the fault signals a return.
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

    let t_start = Instant::now();
    let mut steps: usize = 0;
    let mut distinct_pcs = std::collections::BTreeSet::new();
    let mut hle_calls: std::collections::BTreeMap<u32, usize> = std::collections::BTreeMap::new();
    let mut lv2_calls: std::collections::BTreeMap<u64, usize> = std::collections::BTreeMap::new();
    let mut pc_hits: std::collections::HashMap<u64, u64> = std::collections::HashMap::new();
    const MS_PC_RING_SIZE: usize = 32;
    let mut pc_ring: [u64; MS_PC_RING_SIZE] = [0; MS_PC_RING_SIZE];
    let mut pc_ring_pos: usize = 0;
    // Syscall ring entries: (pc, nr, r3, r4, r5).
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

                    if args[0] == 403 {
                        let buf = args[2] as usize;
                        let len = (args[3] as usize).min(256);
                        let m = rt.memory().as_bytes();
                        // checked_add guards against guest-controlled
                        // buf wrapping on 32-bit usize hosts.
                        let end = buf.checked_add(len);
                        if end.is_some_and(|e| e <= m.len()) {
                            let text = String::from_utf8_lossy(&m[buf..buf + len]);
                            print!("  module_start TTY: {text}");
                        } else {
                            eprintln!(
                                "  module_start TTY dropped: buf=0x{:x}+0x{:x} exceeds guest memory (0x{:x})",
                                buf,
                                len,
                                m.len()
                            );
                        }
                    }
                }

                if steps.is_multiple_of(10_000) {
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

                if let Err(e) = rt.commit_step(&step.result, &step.effects) {
                    // Stepping further would run against state the
                    // computation never committed.
                    eprintln!("  module_start commit_step FAILED at step {steps}: {e:?}");
                    break format!("COMMIT_ERR {e:?} after {steps} steps");
                }

                if let Some(fault) = &step.result.fault {
                    let fault_pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    let guest_code = match fault {
                        cellgov_effects::FaultKind::Guest(c) => Some(*c),
                        _ => None,
                    };

                    // Clean-return detection: PC=0, LR=0, decode-error
                    // fault. The LR=0 check rejects corrupted call
                    // targets that happen to jump to PC=0 after an
                    // intermediate bl overwrote LR.
                    let lr_at_fault = step
                        .result
                        .local_diagnostics
                        .fault_regs
                        .as_ref()
                        .map(|r| r.lr)
                        .unwrap_or(u64::MAX);
                    if fault_pc == 0
                        && lr_at_fault == 0
                        && guest_code
                            .is_some_and(|c| (c & 0xFFFF_0000) == cellgov_ppu::FAULT_DECODE_ERROR)
                    {
                        break format!("RETURNED after {} steps", steps);
                    }
                    let code_str = guest_code
                        .map(|c| format!("0x{c:08x}"))
                        .unwrap_or_else(|| format!("{fault:?}"));
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
                    let raw_str = match fetch_raw_at(&rt, fault_pc) {
                        Some(w) => format!("0x{w:08x}"),
                        None => "<unmapped>".to_string(),
                    };
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
                    let entries = pc_ring_pos.min(MS_PC_RING_SIZE);
                    if entries > 0 {
                        println!("  module_start last {} PCs before fault:", entries);
                        for i in 0..entries {
                            let idx =
                                (pc_ring_pos + MS_PC_RING_SIZE - entries + i) % MS_PC_RING_SIZE;
                            let pc = pc_ring[idx];
                            let (raw, name) = match fetch_raw_at(&rt, pc) {
                                Some(w) => (
                                    format!("0x{w:08x}"),
                                    cellgov_ppu::decode::decode(w)
                                        .ok()
                                        .map(|insn| insn.variant_name().to_string())
                                        .unwrap_or_else(|| "<baddec>".into()),
                                ),
                                None => ("<unmapped>".to_string(), "<unmapped>".to_string()),
                            };
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
            Err(StepError::NoRunnableUnit) | Err(StepError::AllBlocked) => {
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

    if !pc_hits.is_empty() {
        println!("  module_start top PCs by hit count:");
        let mut sorted: Vec<_> = pc_hits.iter().collect();
        sorted.sort_by_key(|&(_, c)| std::cmp::Reverse(*c));
        for (pc, count) in sorted.iter().take(20) {
            let (raw, disasm) = match fetch_raw_at(&rt, **pc) {
                Some(w) => (
                    format!("0x{w:08x}"),
                    cellgov_ppu::decode::decode(w)
                        .ok()
                        .map(|insn| insn.variant_name().to_string())
                        .unwrap_or_else(|| "<baddec>".into()),
                ),
                None => ("<unmapped>".to_string(), "<unmapped>".to_string()),
            };
            println!("    {count:>10}x  PC=0x{:08x}  raw={raw}  {disasm}", **pc);
        }
    }

    rt.into_memory()
}

/// Load liblv2.prx and re-patch GOT entries for every exported NID.
///
/// Returns `None` when the firmware directory is not configured or
/// liblv2.prx is absent; boot continues on pure HLE. Imports in the
/// HLE-keep list stay bound to their trampolines even when the PRX
/// exports them.
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

    // Default placement: first free 64K-aligned page past the HLE
    // trampoline area, reproducing what the LV2 first-fit allocator
    // returns on a fresh boot. CELLGOV_PRX_BASE overrides for pinned
    // microtest and cross-runner layouts.
    let prx_base = match std::env::var("CELLGOV_PRX_BASE") {
        Ok(s) => {
            let trimmed = s.trim();
            let stripped = trimmed
                .strip_prefix("0x")
                .or_else(|| trimmed.strip_prefix("0X"))
                .unwrap_or(trimmed);
            u64::from_str_radix(stripped, 16)
                .unwrap_or_else(|e| die(&format!("CELLGOV_PRX_BASE={s:?}: not a hex u64 ({e})")))
        }
        Err(_) => {
            let tramp_end = tramp_base as u64 + (hle_bindings.len() as u64) * 24;
            (tramp_end + 0xFFFF) & !0xFFFF
        }
    };

    let loaded = match cellgov_ppu::sprx::load_prx(&parsed, mem, prx_base) {
        Ok(l) => l,
        Err(e) => {
            println!("prx: failed to load liblv2.prx at 0x{prx_base:x}: {e:?}");
            return None;
        }
    };

    // NIDs kept on HLE trampolines even when PRX exports them: real
    // implementations depend on module_start initialization that may
    // not complete. Shared with `dump-imports`.
    let hle_keep_nids = cellgov_ppu::prx::HLE_IMPLEMENTED_NIDS;

    // Four disjoint per-binding outcomes whose sum equals
    // hle_bindings.len(): resolved (GOT patched), failed_patch
    // (commit error), kept_hle (on keep list), no_export.
    let mut resolved = 0;
    let mut failed_patch = 0;
    let mut kept_hle = 0;
    let mut no_export = 0;
    for binding in hle_bindings {
        if hle_keep_nids.contains(&binding.nid) {
            kept_hle += 1;
            continue;
        }
        let Some(&real_opd_addr) = loaded.exports.get(&binding.nid) else {
            no_export += 1;
            continue;
        };
        let opd_addr_u32 = real_opd_addr as u32;
        let got_range =
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(binding.stub_addr as u64), 4);
        match got_range {
            Some(range) => match mem.apply_commit(range, &opd_addr_u32.to_be_bytes()) {
                Ok(()) => resolved += 1,
                Err(e) => {
                    failed_patch += 1;
                    eprintln!(
                        "prx: GOT patch at 0x{:08x} (nid 0x{:08x}) FAILED ({e:?}); import stays on HLE",
                        binding.stub_addr, binding.nid
                    );
                }
            },
            None => {
                failed_patch += 1;
                eprintln!(
                    "prx: GOT patch at 0x{:08x} (nid 0x{:08x}): invalid byte range, import stays on HLE",
                    binding.stub_addr, binding.nid
                );
            }
        }
    }

    println!(
        "prx: loaded {} -- {} exports, {}/{} resolved to real code, {} kept as HLE, {} not exported",
        loaded.name,
        loaded.exports.len(),
        resolved,
        hle_bindings.len(),
        kept_hle,
        no_export,
    );
    if failed_patch > 0 {
        eprintln!(
            "prx: {failed_patch} GOT patch failure(s); those imports remain bound to HLE trampolines"
        );
    }

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
