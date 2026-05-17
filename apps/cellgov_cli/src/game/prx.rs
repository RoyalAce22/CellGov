//! Firmware PRX loading, module_start execution, and TLS pre-init for `run-game`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Instant;

use cellgov_core::{Runtime, RuntimeMode, StepError};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, StagedWrite, StagingMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::process_address_space::PS3_PRIMARY_STACK_BASE;
use cellgov_time::Budget;

use super::boot::HLE_HEAP_BASE;
use super::diag::{append_syscall_ring, fetch_raw_at, format_fault};
use super::step_loop::tty::{classify_tty_capture, TtyCaptureDecision};
use super::step_loop::{RingCursor, PC_RING_SIZE, SYSCALL_RING_SIZE};
use crate::cli::exit::die;

pub(super) struct PrxLoadInfo {
    pub(super) name: String,
    /// Filesystem stem of the source PRX (e.g. `"libaudio"` for
    /// `libaudio.sprx`); empty when no source path is available.
    pub(super) stem: String,
    pub(super) base: u64,
    /// Exclusive end of the loaded data segment. `alloc_base`
    /// must clear `max(data_end)` across all loaded PRXs or
    /// `sys_memory_allocate` hands out addresses inside a PRX.
    pub(super) data_end: u64,
    pub(super) toc: u64,
    pub(super) relocs_applied: usize,
    pub(super) module_start: Option<cellgov_ppu::sprx::LoadedOpd>,
    pub(super) module_stop: Option<cellgov_ppu::sprx::LoadedOpd>,
}

/// Foundation closure for firmware-set boot. Mirrors
/// `apps/cellgov_cli/tests/firmware_set_load.rs::FOUNDATION_STEMS`;
/// keep the two in sync.
const FOUNDATION_STEMS: &[&str] = &[
    "libaudio",
    "libfiber",
    "libfs",
    "libgcm_sys",
    "libio",
    "liblv2",
    "libnet",
    "libnetctl",
    "libspurs_jq",
    "libsre",
    "libsync2",
    "libsysmodule",
    "libsysutil",
    "libsysutil_np",
];

/// A second [`cellgov_ppu::prx::HleBinding`] reuses an existing index;
/// the table is malformed by the binder before it reached this map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct DuplicateHleBindingIndex {
    pub index: u32,
    pub prev_nid: u32,
    pub new_nid: u32,
}

impl std::fmt::Display for DuplicateHleBindingIndex {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "duplicate HleBinding index {}: previous nid 0x{:08x} \
             replaced by 0x{:08x} (upstream binder built a malformed table)",
            self.index, self.prev_nid, self.new_nid
        )
    }
}

/// Build an HLE-index to NID map for fast runtime lookup.
///
/// # Errors
///
/// Returns [`DuplicateHleBindingIndex`] when two bindings share an
/// index; the second binding would otherwise silently overwrite the
/// first and dispatch a different NID than the index implies.
pub(super) fn build_nid_map(
    bindings: &[cellgov_ppu::prx::HleBinding],
) -> Result<BTreeMap<u32, u32>, DuplicateHleBindingIndex> {
    let mut map = BTreeMap::new();
    for b in bindings {
        if let Some(prev) = map.insert(b.index, b.nid) {
            return Err(DuplicateHleBindingIndex {
                index: b.index,
                prev_nid: prev,
                new_nid: b.nid,
            });
        }
    }
    Ok(map)
}

/// Must match the HLE `sys_initialize_tls` allocation in `cellgov_core::hle`.
pub(super) const TLS_BASE: u64 = 0x10400000;

/// Guest address of the synthetic kernel-context OPD installed by
/// [`install_kernel_context_opd`]. Sits in the last 16 bytes of the
/// 64 KB TLS reservation, immediately below `HLE_HEAP_BASE`.
const KERNEL_CTX_OPD_ADDR: u64 = 0x1040_FFF0;

// Boundary invariants for the kernel-context OPD slot.
const _: () = assert!(KERNEL_CTX_OPD_ADDR > TLS_BASE);
const _: () = assert!(KERNEL_CTX_OPD_ADDR + 16 == HLE_HEAP_BASE as u64);

/// Offset (from `TLS_BASE`) at which the PT_TLS template starts; PS3
/// kernel convention leaves `0x30` bytes of scratch for the per-thread
/// TLS header that the runtime writes via `sys_initialize_tls`.
const TLS_TEMPLATE_OFFSET: u64 = 0x30;

/// Pre-initialize TLS from the ELF's PT_TLS segment.
///
/// Stages the template bytes and any BSS tail into a single buffer,
/// then commits with one `apply_commit` so the guest never observes a
/// partially initialized TLS image. PS3 LV2 performs this during
/// process creation before any module_start runs.
pub(super) fn pre_init_tls(elf_data: &[u8], mem: &mut GuestMemory) {
    let tls = match cellgov_ppu::loader::find_tls_segment(elf_data) {
        Some(t) => t,
        None => return,
    };

    let p_vaddr = tls.vaddr as usize;
    let p_filesz = tls.filesz as usize;
    let p_memsz = tls.memsz as usize;
    if p_memsz == 0 {
        return;
    }

    // Reject a PT_TLS that would extend into the kernel-context OPD
    // slot at the top of the reservation: the OPD commit happens later
    // and would clobber the template tail otherwise.
    let opd_offset = (KERNEL_CTX_OPD_ADDR - TLS_BASE) as usize;
    let template_end_offset = TLS_TEMPLATE_OFFSET as usize + p_memsz;
    if template_end_offset > opd_offset {
        die(&format!(
            "tls: PT_TLS memsz=0x{p_memsz:x} extends past offset 0x{opd_offset:x} \
             (kernel-context OPD slot); shrink the template or relocate the OPD"
        ));
    }

    let m_len = mem.as_bytes().len();
    let tls_data_start = TLS_BASE as usize + TLS_TEMPLATE_OFFSET as usize;

    let src_end = p_vaddr.checked_add(p_filesz).unwrap_or_else(|| {
        die(&format!(
            "tls: PT_TLS vaddr=0x{p_vaddr:x} + filesz=0x{p_filesz:x} overflows usize"
        ))
    });
    if src_end > m_len {
        die(&format!(
            "tls: PT_TLS src 0x{p_vaddr:x}+0x{p_filesz:x} exceeds guest memory ({m_len} bytes)"
        ));
    }
    let dst_end = tls_data_start.checked_add(p_memsz).unwrap_or_else(|| {
        die(&format!(
            "tls: TLS dst 0x{tls_data_start:x} + memsz=0x{p_memsz:x} overflows usize"
        ))
    });
    if dst_end > m_len {
        die(&format!(
            "tls: TLS dst 0x{tls_data_start:x}+0x{p_memsz:x} exceeds guest memory ({m_len} bytes)"
        ));
    }

    let mut image = vec![0u8; p_memsz];
    if p_filesz > 0 {
        let m = mem.as_bytes();
        image[..p_filesz].copy_from_slice(&m[p_vaddr..src_end]);
    }
    let range = ByteRange::new(GuestAddr::new(tls_data_start as u64), p_memsz as u64)
        .unwrap_or_else(|| die(&format!("tls: invalid byte range at 0x{tls_data_start:x}")));
    mem.apply_commit(range, &image).unwrap_or_else(|e| {
        die(&format!(
            "tls: pre-init commit at 0x{tls_data_start:x} FAILED ({e:?}); TLS not initialized"
        ))
    });

    println!(
        "tls: pre-initialized from PT_TLS at 0x{:x} (filesz=0x{:x}, memsz=0x{:x}) -> 0x{:x}",
        p_vaddr, p_filesz, p_memsz, TLS_BASE
    );
}

/// Write a `{code, toc}` OPD whose body is a single `blr` and return
/// its address. liblv2's entry expects kernel-side function OPDs in
/// r11 / r12; the synthetic OPD lets those calls return cleanly.
fn install_kernel_context_opd(mem: &mut GuestMemory) -> u64 {
    let opd_addr = KERNEL_CTX_OPD_ADDR;
    let blr_addr = (opd_addr as u32) + 8;
    let mut bytes = [0u8; 16];
    bytes[0..4].copy_from_slice(&blr_addr.to_be_bytes());
    bytes[4..8].copy_from_slice(&0u32.to_be_bytes());
    bytes[8..12].copy_from_slice(&0x4e80_0020u32.to_be_bytes());
    let range = ByteRange::new(GuestAddr::new(opd_addr), 16).expect("range");
    if let Err(e) = mem.apply_commit(range, &bytes) {
        die(&format!(
            "module_start: kernel-context OPD install at 0x{opd_addr:x} FAILED ({e:?}); \
             liblv2 module_start would fault on the entry r11/r12 path"
        ));
    }
    opd_addr
}

/// Run a PRX module's module_start to completion or fault.
///
/// A decode-error fault at PC=0 with LR=0 at fault time is treated
/// as a clean return (the LR=0 sentinel set before entry).
pub(super) fn run_module_start(
    mut mem: GuestMemory,
    prx_info: &PrxLoadInfo,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
    max_steps: usize,
    alloc_base: u32,
) -> GuestMemory {
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

    let kctx_opd = install_kernel_context_opd(&mut mem);

    let mut ms_state = cellgov_ppu::state::PpuState::new();
    ms_state.pc = ms.code;
    ms_state.gpr[2] = ms.toc;
    // Offset below the game's stack_top so the two cannot collide
    // if a future caller runs them concurrently.
    ms_state.gpr[1] = PS3_PRIMARY_STACK_BASE + 0x8000;
    ms_state.gpr[11] = kctx_opd;
    ms_state.gpr[12] = kctx_opd;
    // PPC64 convention: r13 = TLS_area + 0x7030.
    ms_state.gpr[13] = TLS_BASE + 0x30 + 0x7000;
    // LR=0 sentinel: blr from module_start jumps to PC=0, where the
    // all-zero word fails to decode and the fault signals a return.
    ms_state.lr = 0;

    let mut rt = Runtime::new(mem, Budget::new(1), max_steps);
    rt.set_mode(RuntimeMode::FaultDriven);
    rt.set_hle_heap_base(HLE_HEAP_BASE);
    let nid_map = build_nid_map(hle_bindings).unwrap_or_else(|e| die(&e.to_string()));
    rt.set_hle_nids(nid_map);
    rt.lv2_host_mut().set_mem_alloc_base(alloc_base);
    rt.registry_mut().register_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = ms_state;
        unit
    });

    // Wall-clock display only, not ordering: never feeds
    // `sync_state_hash` or any scheduling decision.
    let t_start = Instant::now();
    let mut steps: usize = 0;
    let mut distinct_pcs = std::collections::BTreeSet::new();
    let mut hle_calls: BTreeMap<u32, usize> = BTreeMap::new();
    let mut lv2_calls: BTreeMap<u64, usize> = BTreeMap::new();
    let mut pc_hits: BTreeMap<u64, u64> = BTreeMap::new();
    let mut pc_ring: [u64; PC_RING_SIZE] = [0; PC_RING_SIZE];
    let mut pc_cursor = RingCursor::new(PC_RING_SIZE);
    // (nr, pc) per the shared append_syscall_ring schema.
    let mut sc_ring: [(u64, u64); SYSCALL_RING_SIZE] = [(0, 0); SYSCALL_RING_SIZE];
    let mut sc_cursor = RingCursor::new(SYSCALL_RING_SIZE);

    let outcome: String = loop {
        match rt.step() {
            Ok(step) => {
                steps += 1;

                if let Some(pc) = step.result.local_diagnostics.pc {
                    distinct_pcs.insert(pc);
                    *pc_hits.entry(pc).or_insert(0) += 1;
                    let idx = pc_cursor.record();
                    pc_ring[idx] = pc;
                }

                if let Some(args) = &step.result.syscall_args {
                    if args[0] >= 0x10000 {
                        let idx = (args[0] - 0x10000) as u32;
                        *hle_calls.entry(idx).or_insert(0) += 1;
                    } else {
                        *lv2_calls.entry(args[0]).or_insert(0) += 1;
                    }
                    let sc_pc = step.result.local_diagnostics.pc.unwrap_or(0);
                    let idx = sc_cursor.record();
                    sc_ring[idx] = (args[0], sc_pc);

                    if args[0] == cellgov_ps3_abi::syscall::TTY_WRITE {
                        match classify_tty_capture(args, rt.memory().as_bytes()) {
                            TtyCaptureDecision::InBounds { bytes, .. } => {
                                let preview = &bytes[..bytes.len().min(256)];
                                let text = String::from_utf8_lossy(preview);
                                print!("  module_start TTY: {text}");
                            }
                            TtyCaptureDecision::Oob { buf, len, mem_len } => {
                                eprintln!(
                                    "  module_start TTY dropped: buf=0x{buf:x}+0x{len:x} exceeds guest memory (0x{mem_len:x})"
                                );
                            }
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

                    // LR=0 check rejects corrupted call targets that
                    // happen to jump to PC=0 after an intermediate bl
                    // overwrote LR.
                    let lr_at_fault = step
                        .result
                        .local_diagnostics
                        .fault_regs
                        .as_ref()
                        .map(|r| r.lr)
                        .unwrap_or(u64::MAX);
                    if fault_pc == 0
                        && lr_at_fault == 0
                        && guest_code.is_some_and(cellgov_ppu::is_decode_error)
                    {
                        break format!("RETURNED after {} steps", steps);
                    }
                    let mut fault_text =
                        format_fault(&rt, &step.result, fault, steps, &pc_ring, &pc_cursor, &[]);
                    // format_fault renders PC ring only; append the
                    // syscall ring so module_start retains the same
                    // signal as a run-game fault.
                    append_syscall_ring(&mut fault_text, &sc_ring, &sc_cursor, hle_bindings);
                    eprintln!("module_start {fault_text}");
                    let code_str = guest_code
                        .map(|c| format!("0x{c:08x}"))
                        .unwrap_or_else(|| format!("{fault:?}"));
                    let raw_str = match fetch_raw_at(&rt, fault_pc) {
                        Some(w) => format!("0x{w:08x}"),
                        None => "<unmapped>".to_string(),
                    };
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
        sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (idx, count) in sorted.iter().take(10) {
            let name = hle_bindings
                .get(**idx as usize)
                .and_then(|b| cellgov_ps3_abi::nid::lookup(b.nid).map(|(m, f)| format!("{m}::{f}")))
                .unwrap_or_else(|| format!("hle_{idx}"));
            println!("    {count:>8}x  {name}");
        }
    }
    if !lv2_calls.is_empty() {
        println!("  module_start LV2 syscalls:");
        let mut sorted: Vec<_> = lv2_calls.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1).then(a.0.cmp(b.0)));
        for (num, count) in sorted.iter().take(10) {
            println!("    {count:>8}x  syscall {num}");
        }
    }

    if !pc_hits.is_empty() {
        println!("  module_start top PCs by hit count:");
        let mut sorted: Vec<_> = pc_hits.iter().collect();
        // Tie-break by PC so the ranking is independent of iteration order.
        sorted.sort_by(|&(pc_a, c_a), &(pc_b, c_b)| c_b.cmp(c_a).then(pc_a.cmp(pc_b)));
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

/// Locate the firmware module file for `stem` under `dir_path`.
///
/// Prefers `.sprx` (SCE-wrapped) over `.prx` (pre-decrypted) so both
/// boot modes converge on the same on-disk file when both exist.
fn find_firmware_module(dir_path: &Path, stem: &str) -> Option<PathBuf> {
    let sprx = dir_path.join(format!("{stem}.sprx"));
    if sprx.is_file() {
        return Some(sprx);
    }
    let prx = dir_path.join(format!("{stem}.prx"));
    if prx.is_file() {
        return Some(prx);
    }
    None
}

/// Read a firmware module file and decrypt if SCE-wrapped. Returns
/// the raw bytes otherwise so pre-decrypted `.prx` files load through
/// the same path.
fn read_firmware_module_elf(path: &Path) -> Result<Vec<u8>, String> {
    let raw = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    if raw.len() >= 4 && &raw[..4] == b"SCE\0" {
        cellgov_firmware::sce::decrypt_self_to_elf(&raw)
            .map_err(|e| format!("decrypt {}: {e}", path.display()))
    } else {
        Ok(raw)
    }
}

/// Outcome of an atomic GOT patch batch.
#[derive(Debug, Clone, Copy, Default)]
struct GotPatchStats {
    resolved: usize,
    kept_hle: usize,
    no_export: usize,
}

/// Stage one 4-byte GOT write per binding (HLE-keep and missing-export
/// excluded) and apply the whole batch atomically.
///
/// On any per-item or batch validation failure the staging buffer is
/// dropped and `Err` is returned; guest memory is unchanged so the
/// caller can fall back to pure HLE without a half-patched table.
fn patch_got_atomic(
    bindings: &[cellgov_ppu::prx::HleBinding],
    mem: &mut GuestMemory,
    hle_keep_nids: &[u32],
    mut lookup: impl FnMut(u32) -> Option<u64>,
) -> Result<GotPatchStats, String> {
    let mut staging = StagingMemory::new();
    let mut stats = GotPatchStats::default();
    for binding in bindings {
        if hle_keep_nids.contains(&binding.nid) {
            stats.kept_hle += 1;
            continue;
        }
        let Some(opd_addr) = lookup(binding.nid) else {
            stats.no_export += 1;
            continue;
        };
        let opd_u32 = opd_addr as u32;
        let range =
            ByteRange::new(GuestAddr::new(binding.stub_addr as u64), 4).ok_or_else(|| {
                format!(
                    "GOT slot at 0x{:08x} (nid 0x{:08x}): invalid 4-byte range",
                    binding.stub_addr, binding.nid
                )
            })?;
        staging.stage(StagedWrite {
            range,
            bytes: opd_u32.to_be_bytes().to_vec(),
        });
        stats.resolved += 1;
    }
    staging.drain_into(mem).map_err(|e| {
        format!(
            "GOT batch validation failed ({e:?}); {} staged write(s) discarded",
            stats.resolved
        )
    })?;
    Ok(stats)
}

/// Load liblv2 and re-patch GOT entries for every exported NID.
///
/// Returns `None` when the firmware directory is not configured, the
/// module is absent, or the decrypt / parse / load / GOT-patch step
/// fails; boot continues on pure HLE. Imports in the HLE-keep list
/// stay bound to their trampolines even when the PRX exports them.
pub(super) fn load_firmware_prx(
    firmware_dir: Option<&str>,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
    mem: &mut GuestMemory,
    code_floor: u32,
) -> Option<PrxLoadInfo> {
    let dir = firmware_dir?;
    let dir_path = std::path::PathBuf::from(dir);
    let prx_path = match find_firmware_module(&dir_path, "liblv2") {
        Some(p) => p,
        None => {
            println!(
                "prx: liblv2.sprx / liblv2.prx not found under {}, using pure HLE",
                dir_path.display()
            );
            return None;
        }
    };

    let prx_data = match read_firmware_module_elf(&prx_path) {
        Ok(d) => d,
        Err(e) => {
            println!("prx: {e}");
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

    let prx_base = resolve_prx_base(code_floor);

    let loaded = match cellgov_ppu::sprx::load_prx(&parsed, mem, prx_base) {
        Ok(l) => l,
        Err(e) => {
            println!("prx: failed to load liblv2.prx at 0x{prx_base:x}: {e:?}");
            return None;
        }
    };

    let stats = match patch_got_atomic(
        hle_bindings,
        mem,
        cellgov_ppu::prx::HLE_IMPLEMENTED_NIDS,
        |nid| loaded.exports.get(&nid).copied(),
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("prx: liblv2 GOT patch aborted ({e}); falling back to pure HLE");
            return None;
        }
    };

    println!(
        "prx: loaded {} -- {} exports, {}/{} resolved to real code, {} kept as HLE, {} not exported",
        loaded.name,
        loaded.exports.len(),
        stats.resolved,
        hle_bindings.len(),
        stats.kept_hle,
        stats.no_export,
    );

    Some(PrxLoadInfo {
        name: loaded.name,
        stem: "liblv2".to_string(),
        base: loaded.base,
        data_end: loaded.data_end,
        toc: loaded.toc,
        relocs_applied: loaded.relocs_applied,
        module_start: loaded.module_start,
        module_stop: loaded.module_stop,
    })
}

/// Resolve the PRX placement base, honoring `CELLGOV_PRX_BASE` and
/// falling back to the first 64K-aligned page past the caller-supplied
/// `code_floor` (which must already account for both the HLE
/// trampoline area and any Ps3Spec OPD/body span).
fn resolve_prx_base(code_floor: u32) -> u64 {
    let s = match std::env::var("CELLGOV_PRX_BASE") {
        Ok(s) => s,
        Err(_) => return (code_floor as u64 + 0xFFFF) & !0xFFFF,
    };
    let trimmed = s.trim();
    let stripped = trimmed
        .strip_prefix("0x")
        .or_else(|| trimmed.strip_prefix("0X"))
        .unwrap_or(trimmed);
    let base = u64::from_str_radix(stripped, 16)
        .unwrap_or_else(|e| die(&format!("CELLGOV_PRX_BASE={s:?}: not a hex u64 ({e})")));
    if base & 0xFFFF != 0 {
        die(&format!(
            "CELLGOV_PRX_BASE=0x{base:x}: must be 64K-aligned (low 16 bits zero)"
        ));
    }
    if base < code_floor as u64 {
        die(&format!(
            "CELLGOV_PRX_BASE=0x{base:x}: below code_floor 0x{code_floor:x}"
        ));
    }
    // Main region spans `[0, 0x4000_0000)`; PRX placement above that
    // hits reserved or unmapped regions.
    if base >= 0x4000_0000 {
        die(&format!(
            "CELLGOV_PRX_BASE=0x{base:x}: must be in main region (< 0x4000_0000)"
        ));
    }
    base
}

/// Load the foundation closure via [`cellgov_ppu::prx_loader::load_firmware_set`],
/// patch the game ELF's HLE bindings against the resulting union export
/// table (honoring [`cellgov_ppu::prx::HLE_IMPLEMENTED_NIDS`] keep-list),
/// and return one [`PrxLoadInfo`] per module in topological order.
///
/// Returns an empty vector when the firmware directory is absent, a
/// foundation stem is missing, or any decrypt / parse / load / GOT-
/// patch step fails. The caller falls back to pure HLE.
pub(super) fn load_firmware_set_bound(
    firmware_dir: Option<&str>,
    hle_bindings: &[cellgov_ppu::prx::HleBinding],
    mem: &mut GuestMemory,
    code_floor: u32,
) -> Vec<PrxLoadInfo> {
    let Some(dir) = firmware_dir else {
        println!("prx: firmware-set mode requires --firmware-dir; pure HLE in use");
        return Vec::new();
    };
    let dir_path = std::path::PathBuf::from(dir);

    let mut bytes_by_path: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    // id_to_stem feeds the boot-side Lv2Host PRX registry so
    // firmware-side `_sys_prx_load_module(path)` can resolve guest
    // paths back to a kernel id.
    let mut id_to_stem: BTreeMap<cellgov_ppu::prx_loader::PrxModuleId, String> = BTreeMap::new();
    let mut missing: Vec<&str> = Vec::new();
    for stem in FOUNDATION_STEMS {
        let path = match find_firmware_module(&dir_path, stem) {
            Some(p) => p,
            None => {
                missing.push(*stem);
                continue;
            }
        };
        let elf = match read_firmware_module_elf(&path) {
            Ok(d) => d,
            Err(e) => {
                println!("prx: {e}");
                return Vec::new();
            }
        };
        // Pull module_id up front so the post-load image.loaded map
        // can be keyed back to the file stem (the registry is keyed
        // by stem since cellSysmoduleLoadModule passes guest paths).
        match cellgov_ppu::sprx::parse_prx(&elf) {
            Ok(parsed) => {
                id_to_stem.insert(parsed.module_id, (*stem).to_string());
            }
            Err(e) => {
                println!("prx: failed to parse {}: {e:?}", path.display());
                return Vec::new();
            }
        }
        let path_str = match path.to_str() {
            Some(s) => s.to_string(),
            None => {
                println!("prx: non-utf8 firmware path: {}", path.display());
                return Vec::new();
            }
        };
        bytes_by_path.insert(path_str, elf);
    }
    if !missing.is_empty() {
        println!(
            "prx: firmware-set mode: foundation stems missing under {}: {missing:?}; pure HLE",
            dir_path.display()
        );
        return Vec::new();
    }

    let prx_base = resolve_prx_base(code_floor);

    let image = match cellgov_ppu::prx_loader::load_firmware_set(bytes_by_path, mem, prx_base) {
        Ok(img) => img,
        Err(e) => {
            println!("prx: firmware-set load failed at base 0x{prx_base:x}: {e:?}; pure HLE");
            return Vec::new();
        }
    };

    let stats = match patch_got_atomic(
        hle_bindings,
        mem,
        cellgov_ppu::prx::HLE_IMPLEMENTED_NIDS,
        |nid| image.export_table.get(nid),
    ) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("prx: firmware-set GOT patch aborted ({e}); falling back to pure HLE");
            return Vec::new();
        }
    };
    println!(
        "prx: firmware-set loaded {} module(s), {} NIDs in export table, \
         {}/{} game imports resolved to firmware OPDs, {} kept as HLE, {} not exported",
        image.loaded.len(),
        image.export_table.len(),
        stats.resolved,
        hle_bindings.len(),
        stats.kept_hle,
        stats.no_export,
    );

    let mut out: Vec<PrxLoadInfo> = Vec::with_capacity(image.loaded.len());
    for id in &image.topological_order {
        let Some(prx) = image.loaded.get(id) else {
            continue;
        };
        out.push(PrxLoadInfo {
            name: prx.name.clone(),
            stem: id_to_stem.get(id).cloned().unwrap_or_default(),
            base: prx.base,
            data_end: prx.data_end,
            toc: prx.toc,
            relocs_applied: prx.relocs_applied,
            module_start: prx.module_start,
            module_stop: prx.module_stop,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(index: u32, nid: u32) -> cellgov_ppu::prx::HleBinding {
        cellgov_ppu::prx::HleBinding {
            index,
            nid,
            stub_addr: 0,
            module: String::new(),
        }
    }

    #[test]
    fn build_nid_map_accepts_unique_indices() {
        let map = build_nid_map(&[binding(0, 0xAA), binding(1, 0xBB)]).unwrap();
        assert_eq!(map.get(&0), Some(&0xAA));
        assert_eq!(map.get(&1), Some(&0xBB));
    }

    #[test]
    fn build_nid_map_rejects_duplicate_index() {
        let err = build_nid_map(&[binding(0, 0xAA), binding(0, 0xBB)]).unwrap_err();
        assert_eq!(
            err,
            DuplicateHleBindingIndex {
                index: 0,
                prev_nid: 0xAA,
                new_nid: 0xBB,
            }
        );
    }
}
