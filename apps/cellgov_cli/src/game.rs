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
) {
    let t_start = Instant::now();
    let elf_data = load_file_or_die(elf_path);

    // Determine memory size from ELF segments.
    let required_size = cellgov_ppu::loader::required_memory_size(&elf_data)
        .unwrap_or_else(|e| die(&format!("failed to parse ELF: {e:?}")));

    // Round up to 64KB alignment. Guest memory must be large enough for
    // the game, PRX modules, and PS3 user-memory allocations starting
    // at 0x00010000. The flat layout covers both the user-memory
    // region (0x00010000+) and the EBOOT load region (0x10000000+)
    // with a contiguous 1 GB backing.
    let min_for_kernel = 0x4000_0000usize; // covers user + EBOOT regions
    let game_size = ((required_size + 0xFFFF) & !0xFFFF) + 0x200000;
    let mem_size = game_size.max(min_for_kernel);
    let mut state = cellgov_ppu::state::PpuState::new();
    // Build the PS3-spec multi-region layout:
    // - main at base 0 covering the ELF + allocator pool
    // - stack at 0xD0000000 (primary-thread stack)
    // - rsx at 0xC0000000 (video/RSX local memory, reserved
    //   provisional -- reads return zero and are counted; writes
    //   fault until a real RSX implementation lands)
    // - spu_reserved at 0xE0000000 (SPU-shared range, same
    //   provisional semantics)
    // `--strict-reserved` upgrades rsx and spu_reserved to
    // `ReservedStrict`, making reads fault too.
    let reserved_access = if strict_reserved {
        cellgov_mem::RegionAccess::ReservedStrict
    } else {
        cellgov_mem::RegionAccess::ReservedZeroReadable
    };
    let mut mem = cellgov_mem::GuestMemory::from_regions(vec![
        cellgov_mem::Region::new(0, mem_size, "main", cellgov_mem::PageSize::Page64K),
        cellgov_mem::Region::new(
            PS3_PRIMARY_STACK_BASE,
            PS3_PRIMARY_STACK_SIZE,
            "stack",
            cellgov_mem::PageSize::Page4K,
        ),
        cellgov_mem::Region::with_access(
            PS3_RSX_BASE,
            PS3_RSX_SIZE,
            "rsx",
            cellgov_mem::PageSize::Page64K,
            reserved_access,
        ),
        cellgov_mem::Region::with_access(
            PS3_SPU_RESERVED_BASE,
            PS3_SPU_RESERVED_SIZE,
            "spu_reserved",
            cellgov_mem::PageSize::Page64K,
            reserved_access,
        ),
    ])
    .unwrap_or_else(|e| die(&format!("failed to build guest memory layout: {e:?}")));
    let t_mem_alloc = t_start.elapsed();

    // Address 0 must stay zero: CRT0 linked lists use *NULL as a
    // termination sentinel. No exit stub is planted here.

    let load_result = cellgov_ppu::loader::load_ppu_elf(&elf_data, &mut mem, &mut state)
        .unwrap_or_else(|e| die(&format!("failed to load ELF: {e:?}")));
    let t_elf_load = t_start.elapsed();

    // Parse and bind HLE import stubs.
    let tramp_base = ((required_size + 0xFFF) & !0xFFF) as u32;
    // CELLGOV_HLE_OPD_BASE: when set (e.g. "0x008A0120"), HLE bindings
    // produce 8-byte OPDs packed at that address (matching RPCS3's
    // `vm::alloc(N*8, vm::main)` HLE-table shape per
    // tools/rpcs3-src/rpcs3/Emu/Cell/PPUModule.cpp). Body trampolines
    // land at `body_base` (16-byte stride). The GOT entries become
    // packed pointers into user memory rather than 24-byte-aligned
    // pointers into the upper region.
    let hle_layout = match std::env::var("CELLGOV_HLE_OPD_BASE") {
        Ok(s) => {
            let opd_base =
                u32::from_str_radix(s.trim_start_matches("0x"), 16).unwrap_or(tramp_base);
            // Body trampolines packed past the OPD area. Each binding
            // gets one 16-byte body. Reserve space assuming 256 max
            // bindings (overestimates flOw's 140; cheap headroom).
            let body_base = opd_base + 256 * 8;
            cellgov_ppu::prx::HleLayout::Ps3Spec {
                opd_base,
                body_base,
            }
        }
        Err(_) => cellgov_ppu::prx::HleLayout::Legacy24,
    };
    let hle_bindings = match cellgov_ppu::prx::parse_imports(&elf_data) {
        Ok(modules) => {
            let bindings = cellgov_ppu::prx::bind_hle_stubs_with_layout(
                &modules, &mut mem, hle_layout, tramp_base,
            );
            println!(
                "imports: {} modules, {} functions bound to HLE stubs",
                modules.len(),
                bindings.len()
            );
            for m in &modules {
                let first_stub = m.functions.first().map(|f| f.stub_addr).unwrap_or(0);
                println!(
                    "  {}: {} functions, first stub at 0x{:x}",
                    m.name,
                    m.functions.len(),
                    first_stub
                );
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
    //
    // Skipped when CELLGOV_SKIP_MODULE_START=1: liblv2's module_start
    // tries to dynamically load further PRX modules via
    // sys_prx_load_module, which CellGov does not implement. liblv2's
    // failure-recovery path scribbles a sentinel through a stale
    // pointer into the game's text segment. Skipping lets the game
    // call into liblv2 functions directly (most are thin wrappers
    // around LV2 syscalls that do not require module_start to have
    // initialized internal state).
    let skip_ms = std::env::var("CELLGOV_SKIP_MODULE_START")
        .map(|v| v == "1")
        .unwrap_or(false);
    if let Some(ref info) = prx_info {
        if !skip_ms {
            mem = run_module_start(mem, info, &hle_bindings, max_steps);
        } else {
            println!("module_start: skipped (CELLGOV_SKIP_MODULE_START=1)");
        }
    }

    // Set up stack at the PS3-spec primary-thread stack region.
    state.gpr[1] = PS3_PRIMARY_STACK_TOP;
    state.lr = 0;

    // Place sys_memory_allocate's first return above the ELF's
    // occupancy in the user-memory region (0x00010000-0x0FFFFFFF).
    // Real PS3 LV2 shares that region between the loaded ELF and the
    // allocator pool; the kernel tracks the ELF's extent so
    // allocations do not overwrite it. We compute the highest
    // user-region PT_LOAD end, round up to 64KB, and hand it to the
    // Lv2Host.
    let user_region_end = elf_user_region_end(&elf_data);
    // Reserve trampoline area when CELLGOV_HLE_OPD_BASE is set so the
    // allocator does not hand out addresses that overwrite the packed
    // OPD/body region. Pack uses 256 OPD slots * 8 bytes + 256 body
    // slots * 16 bytes = 0x1800 bytes total starting at opd_base.
    let trampoline_area_end = match std::env::var("CELLGOV_HLE_OPD_BASE") {
        Ok(s) => u64::from_str_radix(s.trim_start_matches("0x"), 16)
            .ok()
            .map(|opd_base| opd_base + 0x1800)
            .unwrap_or(0),
        Err(_) => 0,
    } as usize;
    let alloc_floor = user_region_end.max(trampoline_area_end);
    let alloc_base = ((alloc_floor + 0xFFFF) & !0xFFFF).max(0x0001_0000) as u32;

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
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 32);
        match range.and_then(|r| mem.read(r)) {
            Some(slice) => {
                let label = mem
                    .containing_region(addr, 32)
                    .map(|r| r.label())
                    .unwrap_or("<unmapped>");
                print!("mem[0x{addr:x}] ({label}):");
                for b in slice {
                    print!(" {b:02x}");
                }
                println!();
            }
            None => println!("mem[0x{addr:x}]: out of range"),
        }
    }

    // Budget=1 gives instruction-level fault granularity: each
    // runtime step executes exactly one PPU instruction.
    let mut rt = Runtime::new(mem, Budget::new(1), max_steps);
    rt.set_mode(RuntimeMode::FaultDriven);
    // Place HLE heap after TLS area (0x10400000 + 64KB for TLS).
    rt.set_hle_heap_base(0x10410000);
    rt.set_hle_nids(build_nid_map(&hle_bindings));
    // sys_memory_allocate hands out from the PS3 user-memory region,
    // above the loaded ELF's footprint.
    rt.lv2_host_mut().set_mem_alloc_base(alloc_base);
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

/// One region in a checkpoint observation manifest, sharing the schema
/// used by `tools/rpcs3_to_observation/` and `tests/fixtures/flow_checkpoint.toml`.
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
fn elf_user_region_end(data: &[u8]) -> usize {
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

#[cfg(test)]
mod tests {
    use super::{elf_user_region_end, CheckpointManifest, CheckpointRegion};

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
            .join("flow_checkpoint.toml");
        let text = std::fs::read_to_string(&path).expect("read");
        let m: CheckpointManifest = toml::from_str(&text).expect("parses");
        assert!(!m.regions.is_empty());
        assert!(m.regions.iter().any(|r| r.name == "code"));
    }
}
