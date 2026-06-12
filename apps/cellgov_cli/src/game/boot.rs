//! Boot preparation shared between `run-game` and `bench-boot`.

use std::time::{Duration, Instant};

use cellgov_core::{default_budget_for_mode, Runtime, RuntimeMode};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

use cellgov_ps3_abi::process_address_space::{
    PS3_CHILD_STACKS_BASE, PS3_CHILD_STACKS_SIZE, PS3_PRIMARY_STACK_BASE, PS3_PRIMARY_STACK_SIZE,
    PS3_PRIMARY_STACK_TOP, PS3_RSX_BASE, PS3_RSX_IOMAP_BASE, PS3_RSX_IOMAP_SIZE, PS3_RSX_SIZE,
    PS3_SPU_RESERVED_BASE, PS3_SPU_RESERVED_SIZE,
};

use super::manifest::TitleManifest;
use super::prx::{
    install_kernel_context_opd, load_firmware_set_bound, pre_init_tls, run_module_start,
    ModuleStartOutcome,
};
use crate::cli::env::parse_env_bool;
use crate::cli::exit::die;

/// Default primary-thread priority when the title's `sys_proc_param`
/// block is absent.
const DEFAULT_PRIMARY_PRIO: u32 = 1001;

/// Seeded ring size per slot. V256 shape: the dispatcher's six
/// non-zero field budgets (56+8+76+4+22+10 = 176) drain inside the
/// 256-byte limit, so module_start's terminal stall is the
/// producer-fed cond[0] record-finish wait, not a mid-record
/// depleted-ring symptom. See
/// `docs/dev/phases/design/phases_41-50/phase_41.md`.
const CELLSYSUTIL_RING_LIMIT: u32 = 256;

/// Boot-state seed for the cellSysutil slot-state shm.
///
/// Models the first producer record an external firmware producer
/// would have delivered before the title ran. Field consumers are
/// the wait-fn guard reads decoded in the phase doc: state@+20
/// (`!= 2` falls through), cursor@+16 vs limit@+4 (`<` enters the
/// drain), read_pos@+8 / write_pos@+12 / data_offset@+0 drive the
/// per-record memcpy, predicate@+30 (`0` avoids the early-exit
/// error path).
pub(super) fn cellsysutil_system_seed() -> cellgov_lv2::SystemStateSeed {
    use cellgov_ps3_abi::system_ipc::{
        CELLSYSUTIL_SHM_IPC_KEY, CELLSYSUTIL_SLOT_COUNT, CELLSYSUTIL_SLOT_CURSOR_OFFSET,
        CELLSYSUTIL_SLOT_DATA_OFFSET, CELLSYSUTIL_SLOT_LIMIT_OFFSET, CELLSYSUTIL_SLOT_STRIDE,
    };
    let mut writes = Vec::new();
    for slot in 0..CELLSYSUTIL_SLOT_COUNT {
        let base = slot * CELLSYSUTIL_SLOT_STRIDE;
        writes.push((base, CELLSYSUTIL_SLOT_DATA_OFFSET.to_be_bytes().to_vec()));
        writes.push((
            base + CELLSYSUTIL_SLOT_LIMIT_OFFSET,
            CELLSYSUTIL_RING_LIMIT.to_be_bytes().to_vec(),
        ));
        writes.push((base + 8, 0u32.to_be_bytes().to_vec()));
        writes.push((base + 12, CELLSYSUTIL_RING_LIMIT.to_be_bytes().to_vec()));
        writes.push((
            base + CELLSYSUTIL_SLOT_CURSOR_OFFSET,
            0u32.to_be_bytes().to_vec(),
        ));
        writes.push((base + 20, 1u32.to_be_bytes().to_vec()));
        writes.push((base + 30, vec![0u8]));
        writes.push((
            base + CELLSYSUTIL_SLOT_DATA_OFFSET,
            vec![0u8; CELLSYSUTIL_RING_LIMIT as usize],
        ));
    }
    cellgov_lv2::SystemStateSeed {
        shm_ipc_key: CELLSYSUTIL_SHM_IPC_KEY,
        writes,
    }
}

/// Bump-arena base for HLE-side allocations, above the TLS scratch
/// at `0x10400000`.
pub const HLE_HEAP_BASE: u32 = 0x10410000;

/// Narrow `u64` to `u32` at the host/guest boundary; dies on
/// overflow with `label` named.
fn u32_or_die(label: &str, value: u64) -> u32 {
    u32::try_from(value)
        .unwrap_or_else(|_| die(&format!("{label}: 0x{value:x} does not fit in u32")))
}

/// `--strict-reserved` plus `rsx_mirror = true` is unsatisfiable:
/// strict-reserved forces RSX `ReservedStrict` (rejects all writes),
/// rsx_mirror projects flip-status bytes into that same region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub(super) enum StrictReservedConflict {
    #[error(
        "boot: --strict-reserved conflicts with title manifest rsx_mirror=true; \
         rsx_mirror requires a writable RSX region but --strict-reserved forces \
         it ReservedStrict. Drop one of the two."
    )]
    RsxMirror,
}

pub(super) fn check_strict_reserved_vs_rsx_mirror(
    strict_reserved: bool,
    rsx_mirror: bool,
) -> Result<(), StrictReservedConflict> {
    if strict_reserved && rsx_mirror {
        return Err(StrictReservedConflict::RsxMirror);
    }
    Ok(())
}

/// Walk the title ELF's executable PT_LOAD segments through the
/// PPU decoder and print the gap report. Firmware PRX text is out
/// of scope; gaps there surface at execution time.
fn emit_prescan_report(elf_data: &[u8], elf_path: &str) {
    let (report, coverage) = match cellgov_ppu::prescan::scan_elf_text(elf_data) {
        Ok(pair) => pair,
        Err(e) => {
            eprintln!("prescan: skipped, {e} in {elf_path}");
            return;
        }
    };
    for line in super::prescan_format::format_prescan_report(&report, &coverage, elf_path) {
        eprintln!("{line}");
    }
}

fn content_source_label(
    source: &super::content::ContentBaseSource,
    manifest_base: &str,
    override_base: Option<&std::path::Path>,
) -> String {
    use super::content::ContentBaseSource;
    match source {
        ContentBaseSource::Manifest => format!("manifest base ({manifest_base})"),
        ContentBaseSource::Usrdir { path } => {
            format!("EBOOT-adjacent USRDIR ({})", path.display())
        }
        ContentBaseSource::Override { env } => format!(
            "override env {env}={}",
            override_base
                .map(|p| p.display().to_string())
                .unwrap_or_default(),
        ),
    }
}

pub(super) struct PreparedBoot {
    pub rt: Runtime,
    pub elf_data: Vec<u8>,
    pub timings: StartupTimings,
    /// Per-step budget resolved during `prepare`.
    pub step_budget: Budget,
}

#[derive(Debug, Clone, Copy, Default)]
pub(super) struct StartupTimings {
    pub mem_alloc: Duration,
    pub elf_load: Duration,
    pub hle_bind: Duration,
    pub prx_load: Duration,
}

impl StartupTimings {
    pub fn total(&self) -> Duration {
        self.mem_alloc + self.elf_load + self.hle_bind + self.prx_load
    }
}

pub(super) struct PrepareOptions<'a> {
    pub title: &'a TitleManifest,
    pub elf_path: &'a str,
    /// Already-decrypted ELF bytes.
    pub elf_data: Vec<u8>,
    pub firmware_dir: Option<&'a str>,
    pub strict_reserved: bool,
    pub dump_at_pc: Option<u64>,
    pub dump_skip: u32,
    pub print_banner: bool,
    pub profile_pairs: bool,
    pub runtime_max_steps: usize,
    /// Applied after every `module_start` has completed, before the
    /// title's primary unit registers.
    pub patch_bytes: &'a [(u64, u8)],
    pub dump_mem_boot_addrs: &'a [u64],
    pub budget_override: Option<Budget>,
    /// When true, switch runtime mode to `DeterminismCheck` so
    /// per-step `PpuStateHash` records land in the trace buffer.
    pub capture_state_trace: bool,
    /// When true, walk the title ELF's executable PT_LOAD segments
    /// through the PPU decoder before execution and print the gap
    /// report to stderr.
    pub prescan: bool,
}

/// Debug toggles captured by both the primary-thread `register_with`
/// and the `set_ppu_factory` closures so children spawned via
/// `sys_ppu_thread_create` inherit them.
#[derive(Debug, Clone, Copy)]
struct BootDebugOptions {
    dump_at_pc: Option<u64>,
    dump_skip: u32,
    profile_pairs: bool,
}

pub(super) fn prepare(opts: PrepareOptions<'_>) -> PreparedBoot {
    let t_start = Instant::now();
    let elf_data = opts.elf_data;

    let required_size = cellgov_ppu::loader::required_memory_size(&elf_data)
        .unwrap_or_else(|e| die(&format!("failed to parse ELF: {e:?}")));

    // Main region spans the user-memory region (0x00010000+) and the EBOOT
    // load region (0x10000000+) as one contiguous backing; 64KB alignment
    // plus 2 MiB headroom for PRX.
    let min_for_kernel = 0x4000_0000usize;
    let game_size = required_size
        .checked_add(0xFFFF)
        .map(|v| v & !0xFFFF)
        .and_then(|v| v.checked_add(0x200000))
        .unwrap_or_else(|| {
            die(&format!(
                "required_size=0x{required_size:x} overflows usize"
            ))
        });
    let mem_size = game_size.max(min_for_kernel);
    if parse_env_bool("CELLGOV_BOOT_TRACE_MEM") {
        eprintln!(
            "boot: required_size=0x{required_size:x} game_size=0x{game_size:x} \
             floor=0x{min_for_kernel:x} mem_size=0x{mem_size:x} ({:.2} GiB)",
            mem_size as f64 / (1024.0 * 1024.0 * 1024.0),
        );
    }
    let mut state = cellgov_ppu::state::PpuState::new();
    if let Err(err) =
        check_strict_reserved_vs_rsx_mirror(opts.strict_reserved, opts.title.rsx_mirror())
    {
        die(&err.to_string());
    }
    let reserved_access = if opts.strict_reserved {
        cellgov_mem::RegionAccess::ReservedStrict
    } else {
        cellgov_mem::RegionAccess::ReservedZeroReadable
    };
    let rsx_access = if opts.strict_reserved {
        reserved_access
    } else if opts.title.rsx_mirror() {
        cellgov_mem::RegionAccess::ReadWrite
    } else {
        reserved_access
    };
    // Main must end at or below PS3_RSX_IOMAP_BASE so the iomap
    // region the title later writes through stays disjoint.
    if mem_size as u64 > PS3_RSX_IOMAP_BASE {
        die(&format!(
            "boot: required_size 0x{required_size:x} requires main mem_size \
             0x{mem_size:x} which exceeds PS3_RSX_IOMAP_BASE 0x{PS3_RSX_IOMAP_BASE:x}"
        ));
    }
    let mut mem = cellgov_mem::GuestMemory::from_regions(vec![
        cellgov_mem::Region::new(0, mem_size, "main", cellgov_mem::PageSize::Page64K),
        cellgov_mem::Region::new(
            PS3_RSX_IOMAP_BASE,
            PS3_RSX_IOMAP_SIZE,
            "rsx_iomap",
            cellgov_mem::PageSize::Page64K,
        ),
        cellgov_mem::Region::new(
            PS3_PRIMARY_STACK_BASE,
            PS3_PRIMARY_STACK_SIZE,
            "stack",
            cellgov_mem::PageSize::Page4K,
        ),
        cellgov_mem::Region::new(
            PS3_CHILD_STACKS_BASE,
            PS3_CHILD_STACKS_SIZE,
            "child_stacks",
            cellgov_mem::PageSize::Page4K,
        ),
        cellgov_mem::Region::with_access(
            PS3_RSX_BASE,
            PS3_RSX_SIZE,
            "rsx",
            cellgov_mem::PageSize::Page64K,
            rsx_access,
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

    let load_result = cellgov_ppu::loader::load_ppu_elf(&elf_data, &mut mem, &mut state)
        .unwrap_or_else(|e| die(&format!("failed to load ELF: {e:?}")));
    let t_elf_load = t_start.elapsed();

    if opts.prescan {
        emit_prescan_report(&elf_data, opts.elf_path);
    }

    let tramp_base = {
        let rounded = required_size.checked_add(0xFFF).unwrap_or_else(|| {
            die(&format!(
                "required_size=0x{required_size:x} + 0xFFF overflows usize"
            ))
        }) & !0xFFF;
        u32_or_die("tramp_base", rounded as u64)
    };

    let modules = cellgov_ppu::prx::parse_imports(&elf_data)
        .unwrap_or_else(|e| die(&format!("imports: parse failed: {e:?}")));
    if opts.print_banner {
        println!("imports: {} modules", modules.len());
        for m in &modules {
            let first_stub = m.functions.first().map(|f| f.stub_addr).unwrap_or(0);
            println!(
                "  {}: {} functions, first stub at 0x{:x}",
                m.name,
                m.functions.len(),
                first_stub
            );
        }
    }
    let t_hle_bind = t_start.elapsed();

    let code_floor = tramp_base;

    let mut prx_modules =
        load_firmware_set_bound(opts.firmware_dir, &modules, &mut mem, code_floor);
    let t_prx_load = t_start.elapsed();
    if opts.firmware_dir.is_some() && prx_modules.is_empty() {
        eprintln!("prx: firmware directory was supplied but no PRX was loaded");
    }
    if prx_modules.is_empty() {
        // No firmware loaded: install trampolines so calls through
        // unresolved imports produce a structured fault.
        if let Some(info) =
            super::prx::install_unresolved_trampolines_only(&modules, &mut mem, code_floor as u64)
        {
            prx_modules.push(info);
        }
    }

    pre_init_tls(&elf_data, &mut mem);

    // Invariant: mem_alloc base must clear:
    //   1. the game's PT_LOAD ranges (`user_region_end`),
    //   2. the HLE trampoline / OPD-body span (`code_floor`),
    //   3. the firmware-set PRX load region (`prx_region_end`).
    let user_region_end = super::observation::elf_user_region_end(&elf_data);
    let prx_region_end: usize = prx_modules
        .iter()
        .map(|p| p.data_end as usize)
        .max()
        .unwrap_or(0);
    let alloc_floor = user_region_end.max(code_floor as usize).max(prx_region_end);
    let alloc_base = {
        let rounded = alloc_floor.checked_add(0xFFFF).unwrap_or_else(|| {
            die(&format!(
                "alloc_floor=0x{alloc_floor:x} + 0xFFFF overflows usize"
            ))
        }) & !0xFFFF;
        u32_or_die("alloc_base", rounded.max(0x0001_0000) as u64)
    };

    // The kernel-context OPD lives in the TLS reservation and is
    // consumed by every PRX's module_start entry (r11/r12). Install
    // once before the module_start loop.
    let kctx_opd = install_kernel_context_opd(&mut mem);

    let tls_info = cellgov_ppu::loader::find_tls_segment(&elf_data);
    let tls_template = cellgov_ppu::loader::extract_tls_template_bytes(&elf_data);
    match (tls_info.is_some(), tls_template.is_some()) {
        (false, true) => die(
            "tls: PT_TLS bytes extractable but find_tls_segment returned None; \
             PT_TLS parsers disagreed and child-thread r13-relative loads would alias undefined memory",
        ),
        (true, false) => die(
            "tls: find_tls_segment found PT_TLS but extract_tls_template_bytes returned None; \
             PT_TLS parsers disagreed and child-thread TLS would be uninitialized",
        ),
        _ => {}
    }
    let proc_param = cellgov_ppu::loader::find_sys_process_param(&elf_data);
    let malloc_pagesize = proc_param.map(|p| p.malloc_pagesize).unwrap_or(0x100000);

    let mode = if opts.capture_state_trace {
        RuntimeMode::DeterminismCheck
    } else {
        RuntimeMode::FaultDriven
    };
    let step_budget = {
        let b = opts
            .budget_override
            .unwrap_or_else(|| default_budget_for_mode(mode));
        if b.is_exhausted() {
            Budget::new(1)
        } else {
            b
        }
    };
    let step_budget_usize = (step_budget.raw() as usize).max(1);
    if opts.runtime_max_steps < step_budget_usize {
        die(&format!(
            "max_steps={} below budget={step_budget}; raise --max-steps or lower --budget",
            opts.runtime_max_steps
        ));
    }
    let adjusted_max_steps = opts.runtime_max_steps / step_budget_usize;

    let primary_prio: u32 = match proc_param.map(|p| p.primary_prio) {
        Some(p) => {
            u32::try_from(p).unwrap_or_else(|_| die(&format!("primary_prio={p} is negative")))
        }
        None => DEFAULT_PRIMARY_PRIO,
    };
    // `primary_stacksize == 0` reads as "use kernel default".
    let primary_stack_size: u32 = match proc_param.map(|p| p.primary_stacksize) {
        Some(want) if (want as usize) > PS3_PRIMARY_STACK_SIZE => die(&format!(
            "primary_stacksize=0x{want:x} exceeds reserved stack region 0x{:x}; raise PS3_PRIMARY_STACK_SIZE",
            PS3_PRIMARY_STACK_SIZE
        )),
        Some(want) if want > 0 => want,
        _ => u32_or_die("PS3_PRIMARY_STACK_SIZE", PS3_PRIMARY_STACK_SIZE as u64),
    };

    if opts.print_banner {
        println!("title: {}", opts.title.display_name());
        println!("elf: {}", opts.elf_path);
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
        for info in &prx_modules {
            println!(
                "prx: {} at 0x{:x} (toc=0x{:x}, {} relocs)",
                info.name, info.base, info.toc, info.relocs_applied,
            );
        }
        println!("max_steps: {}", opts.runtime_max_steps);
        let budget_source = if opts.budget_override.is_some() {
            "override"
        } else {
            "mode-default"
        };
        println!("budget: {step_budget} ({budget_source})");
        println!();
    }

    // Runtime / LV2 host setup happens BEFORE module_start so every
    // PRX's init runs in the same host the title later runs against.
    let mut rt = Runtime::new(mem, step_budget, adjusted_max_steps);
    rt.set_mode(mode);
    rt.lv2_host_mut().set_mem_alloc_base(alloc_base);
    // Plumb the title's recorded SDK version into the LV2 host so
    // `sys_process_get_sdk_version` reports the value cellSysutil's
    // SDK-keyed init dispatcher gates on. Absent param segment leaves
    // the PSL1GHT homebrew sentinel in place; see
    // `docs/dev/bug_investigations/cellsysutil_allblocked_43.md`.
    if let Some(p) = proc_param {
        rt.lv2_host_mut().set_sdk_version(p.sdk_version);
    }
    // Pre-game system state: the cellSysutil slot-state shm arrives
    // seeded with one producer record per slot, applied when the
    // keyed shm is first mapped (sc 337).
    rt.lv2_host_mut()
        .register_system_seed(cellsysutil_system_seed());
    println!(
        "process_param: sdk_version=0x{:08x} ({})",
        proc_param
            .map(|p| p.sdk_version)
            .unwrap_or(cellgov_ps3_abi::elf::SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN),
        if proc_param.is_some() {
            "from sys_proc_param segment"
        } else {
            "absent -- PSL1GHT homebrew sentinel"
        },
    );
    // Cross-module contract: firmware-side `_sys_prx_load_module(path)`
    // resolves the guest path against this registry to recover the
    // kernel id; an empty stem makes the module unreachable from
    // libsysmodule's load worker.
    for info in &prx_modules {
        // The synthetic unresolved-import trampoline pseudo-module
        // has no firmware identity.
        if info.module_start.is_none() && info.module_stop.is_none() && info.stem.is_empty() {
            continue;
        }
        if info.stem.is_empty() {
            die(&format!(
                "prx: module {:?} loaded with empty stem; registry would not reach it via path lookup",
                info.name
            ));
        }
        rt.lv2_host_mut().prx_registry_mut().register(
            info.stem.clone(),
            info.name.clone(),
            u32_or_die("prx base", info.base),
            u32_or_die("prx data_end", info.data_end),
            u32_or_die("prx toc", info.toc),
            info.module_start
                .map(|opd| u32_or_die("prx module_start", opd.code)),
            info.module_stop
                .map(|opd| u32_or_die("prx module_stop", opd.code)),
        );
    }
    if let Some((bytes, memsz, align, vaddr)) = tls_template {
        rt.lv2_host_mut()
            .set_tls_template(cellgov_lv2::TlsTemplate::new(bytes, memsz, align, vaddr));
    }
    let debug_opts = BootDebugOptions {
        dump_at_pc: opts.dump_at_pc,
        dump_skip: opts.dump_skip,
        profile_pairs: opts.profile_pairs,
    };
    rt.set_ppu_factory(move |id, init| {
        let mut unit = PpuExecutionUnit::new(id);
        {
            let state = unit.state_mut();
            state.pc = init.entry_code;
            state.gpr[1] = init.stack_top;
            state.gpr[2] = init.entry_toc;
            state.gpr[3] = init.arg;
            for (i, value) in init.extra_args.iter().enumerate() {
                state.gpr[4 + i] = *value;
            }
            state.gpr[13] = init.tls_base;
            state.lr = init.lr_sentinel;
        }
        if let Some(pc) = debug_opts.dump_at_pc {
            unit.set_break_pc(pc, debug_opts.dump_skip);
        }
        if debug_opts.profile_pairs {
            unit.set_profile_mode(true);
        }
        Box::new(unit)
    });
    // Cell BE convention: args 0..3 map to r3..r6 (arg0 -> r3, etc.).
    rt.set_spu_factory(|id, init| {
        use cellgov_spu::{loader as spu_loader, SpuExecutionUnit};
        let mut unit = SpuExecutionUnit::new(id);
        spu_loader::load_spu_elf(&init.ls_bytes, unit.state_mut())
            .expect("game boot: load_spu_elf on title-provided ELF; failure indicates a bad LV2 thread init");
        unit.state_mut().pc = init.entry_pc;
        unit.state_mut().set_reg_word_splat(1, init.stack_ptr);
        unit.state_mut().set_reg_word_splat(3, init.args[0] as u32);
        unit.state_mut().set_reg_word_splat(4, init.args[1] as u32);
        unit.state_mut().set_reg_word_splat(5, init.args[2] as u32);
        unit.state_mut().set_reg_word_splat(6, init.args[3] as u32);
        Box::new(unit)
    });

    // Resolves `sysSpuImageOpen("/app_home/spu_main.elf")` against
    // an EBOOT sibling.
    if let Some(parent) = std::path::Path::new(opts.elf_path).parent() {
        let spu_candidate = parent.join("spu_main.elf");
        if spu_candidate.exists() {
            let bytes = std::fs::read(&spu_candidate).unwrap_or_else(|e| {
                die(&format!(
                    "run-game: cannot read {}: {e}",
                    spu_candidate.display()
                ))
            });
            rt.lv2_host_mut()
                .content_store_mut()
                .register(b"/app_home/spu_main.elf", bytes);
        }
    }

    // Resolution priority (high to low):
    //   1. override env var
    //   2. EBOOT-relative USRDIR auto-discovery
    //   3. manifest's checked-in base
    if let Some(content) = opts.title.content.as_ref() {
        let workspace_root = std::env::current_dir()
            .unwrap_or_else(|e| die(&format!("cannot read CWD for content base resolution: {e}")));
        let override_base =
            super::content::override_base_from_env(content, |name| std::env::var(name).ok());
        let usrdir_base = std::path::Path::new(opts.elf_path).parent();
        let registration_result = super::content::register_content_blobs(
            content,
            &workspace_root,
            override_base.as_deref(),
            usrdir_base,
            rt.lv2_host_mut(),
        );
        match registration_result {
            Ok(source) => {
                if opts.print_banner {
                    let label =
                        content_source_label(&source, &content.base, override_base.as_deref());
                    println!(
                        "content: registered {} blob(s) from {label}",
                        content.files.len(),
                    );
                }
            }
            Err(e) => die(&format!("content provider failed: {e}")),
        }
    }

    // Mount registration follows content so the FsStore
    // path-existence check wins over mount resolution.
    if !opts.title.mounts.is_empty() {
        let workspace_root = std::env::current_dir()
            .unwrap_or_else(|e| die(&format!("cannot read CWD for mount path resolution: {e}")));
        let n = match super::mounts::register_mounts(
            &opts.title.mounts,
            &workspace_root,
            |name| std::env::var(name).ok(),
            rt.lv2_host_mut(),
        ) {
            Ok(n) => n,
            Err(e) => die(&format!("mount provider failed: {e}")),
        };
        if opts.print_banner {
            println!("mounts: registered {n} mount(s)");
        }
    }

    // Title primary entry state. r1 = stack top, r3..r10 = PS3 LV2
    // process-start convention args. r11 holds the OPD entry,
    // r12 the malloc pagesize, r13 the TLS pointer. Stamped BEFORE
    // the primary unit is registered so module_start aliases bind
    // to the real entry state.
    state.gpr[1] = PS3_PRIMARY_STACK_TOP;
    state.lr = 0;
    state.gpr[3] = 0;
    state.gpr[4] = 0;
    state.gpr[5] = 0;
    state.gpr[6] = 0;
    state.gpr[7] = 0x0100_0000;
    state.gpr[8] = tls_info.map(|t| t.vaddr).unwrap_or(0);
    state.gpr[9] = tls_info.map(|t| t.filesz).unwrap_or(0);
    state.gpr[10] = tls_info.map(|t| t.memsz).unwrap_or(0);
    state.gpr[11] = load_result.entry;
    state.gpr[12] = malloc_pagesize as u64;
    // r13 is the PS3 PPC64 ABI TLS pointer; LV2 seeds it at process
    // creation and sys_initialize_tls does not touch it.
    state.gpr[13] = super::prx::TLS_BASE + 0x7030;

    // Bound the predecode shadow to executable code; built BEFORE
    // module_start so the primary unit registers with it. PRX
    // module_starts only mutate data segments (mutex tables,
    // allocator state) so a pre-module-start shadow over the
    // text-segment ranges is correct for the title's first
    // instruction. Transient module_start units decode on demand.
    let shadow_extent = (alloc_floor as usize).min(rt.memory().as_bytes().len());
    let t_shadow_start = std::time::Instant::now();
    let shadow =
        cellgov_ppu::shadow::PredecodedShadow::build(0, &rt.memory().as_bytes()[..shadow_extent]);
    if parse_env_bool("CELLGOV_RUNGAME_PROFILE") {
        eprintln!(
            "rungame_profile_shadow: PredecodedShadow::build over {shadow_extent} bytes took {:.2}ms \
             (alloc_floor=0x{alloc_floor:08x} user_region_end=0x{user_region_end:08x} \
             code_floor=0x{code_floor:08x} prx_region_end=0x{prx_region_end:08x})",
            t_shadow_start.elapsed().as_secs_f64() * 1000.0
        );
    }

    // Register the title primary unit and seed its PpuThreadId BEFORE
    // module_starts run. Real LV2 attributes module_start syscalls to
    // the calling (primary) PPU thread; transient module_start units
    // alias to this PpuThreadId for caller resolution. The primary is
    // marked non-runnable via the registry status override so the
    // scheduler skips it while module_start units execute.
    let primary_unit_id = rt.registry_mut().register_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = state;
        unit.set_instruction_shadow(shadow);
        if let Some(pc) = debug_opts.dump_at_pc {
            unit.set_break_pc(pc, debug_opts.dump_skip);
        }
        if debug_opts.profile_pairs {
            unit.set_profile_mode(true);
        }
        unit
    });
    rt.registry_mut()
        .set_status_override(primary_unit_id, cellgov_exec::UnitStatus::Blocked);
    rt.lv2_host_mut().seed_primary_ppu_thread(
        primary_unit_id,
        cellgov_lv2::PpuThreadAttrs {
            entry: load_result.entry,
            arg: 0,
            stack_base: u32_or_die("PS3_PRIMARY_STACK_BASE", PS3_PRIMARY_STACK_BASE),
            stack_size: primary_stack_size,
            priority: primary_prio,
            tls_base: tls_info
                .map(|t| u32_or_die("tls vaddr", t.vaddr))
                .unwrap_or(0),
        },
    );
    // Sync-syscall dispatch from aliased transient module_start
    // units resolves via the primary thread record.
    debug_assert!(
        rt.lv2_host()
            .ppu_thread_id_for_unit(primary_unit_id)
            .is_some(),
        "primary PPU thread record missing pre-module-start; alias targets would not resolve",
    );

    // Each PRX's module_start runs on a transient PPU unit aliased
    // to the primary's PpuThreadId. The transient unit Faults at the
    // LR=0 return sentinel; the alias is dropped immediately so the
    // retired UnitId no longer resolves to a thread record.
    let modules_total = prx_modules
        .iter()
        .filter(|p| p.module_start.is_some())
        .count();
    let skip_ms = parse_env_bool("CELLGOV_SKIP_MODULE_START");
    let modules_started = match (prx_modules.is_empty(), skip_ms) {
        (false, false) => {
            let mut completed: usize = 0;
            for info in &prx_modules {
                let runnable_before = rt.registry().runnable_ids().count();
                match run_module_start(&mut rt, info, kctx_opd) {
                    Ok(ModuleStartOutcome::Completed { .. })
                    | Ok(ModuleStartOutcome::HleStubbed) => completed += 1,
                    Ok(ModuleStartOutcome::Skipped) => {}
                    Err(e) => die(&format!("{e}")),
                }
                // Each module_start either Skipped (no unit registered,
                // count unchanged) or Completed (transient unit ended
                // Faulted, count unchanged). The primary's blocked
                // override holds, so the count never grew during the
                // module's sub-loop.
                debug_assert_eq!(
                    rt.registry().runnable_ids().count(),
                    runnable_before,
                    "module_start {} left a runnable unit in the registry",
                    info.name,
                );
            }
            completed
        }
        (false, true) => {
            eprintln!("module_start: skipped (CELLGOV_SKIP_MODULE_START set)");
            0
        }
        (true, true) => {
            eprintln!(
                "module_start: CELLGOV_SKIP_MODULE_START set, but no PRX was loaded -- flag has no effect"
            );
            0
        }
        (true, false) => 0,
    };

    // Override holds for the duration of the module_start loop: the
    // unit's own status is still Runnable, but effective_status must
    // read Blocked.
    debug_assert_eq!(
        rt.registry().effective_status(primary_unit_id),
        Some(cellgov_exec::UnitStatus::Blocked),
        "primary unit effective_status changed during module_start loop",
    );

    // Patches and dump-mem land AFTER module_start so they observe
    // (or override) the same memory the title sees.
    for &(addr, val) in opts.patch_bytes {
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 1)
            .unwrap_or_else(|| die(&format!("patch: byte 0x{addr:x}: invalid address range")));
        rt.memory_mut()
            .apply_commit(range, &[val])
            .unwrap_or_else(|e| {
                die(&format!(
                    "patch: byte 0x{addr:x} = 0x{val:02x} FAILED ({e:?}); target not committed"
                ))
            });
        if opts.print_banner {
            println!("patch: byte 0x{addr:x} = 0x{val:02x}");
        }
    }
    for &addr in opts.dump_mem_boot_addrs {
        match cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 32) {
            None => println!("mem[0x{addr:x}]: invalid address range"),
            Some(r) => match rt.memory().read(r) {
                Some(slice) => {
                    let label = rt
                        .memory()
                        .containing_region(addr, 32)
                        .map(|r| r.label())
                        .unwrap_or("<unmapped>");
                    print!("mem[0x{addr:x}] ({label}):");
                    for b in slice {
                        print!(" {b:02x}");
                    }
                    println!();
                }
                None => println!("mem[0x{addr:x}]: unmapped"),
            },
        }
    }

    // After module_starts complete, the four-byte mutex id at
    // `*LIBLV2_ONCE_MUTEX_SLOT` must either be zero or reference
    // an id present in the LV2 host's mutex table.
    assert_gating_state_coherent_with_host(&rt, !prx_modules.is_empty());
    debug_assert_eq!(
        modules_started, modules_total,
        "module_start: completed {modules_started} of {modules_total} modules",
    );

    // Clear the runtime override so the scheduler can pick the primary
    // on the first `rt.step()` of the title loop.
    rt.registry_mut().clear_status_override(primary_unit_id);

    PreparedBoot {
        rt,
        elf_data,
        timings: StartupTimings {
            mem_alloc: t_mem_alloc,
            elf_load: t_elf_load - t_mem_alloc,
            hle_bind: t_hle_bind - t_elf_load,
            prx_load: t_prx_load - t_hle_bind,
        },
        step_budget,
    }
}

/// Liblv2's once-mutex slot.
const LIBLV2_ONCE_MUTEX_SLOT: u64 = 0x103a49d8;

/// If memory holds a non-zero once-mutex id, that id must exist in
/// the LV2 host's mutex table.
fn assert_gating_state_coherent_with_host(rt: &Runtime, modules_were_loaded: bool) {
    if !modules_were_loaded {
        return;
    }
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(LIBLV2_ONCE_MUTEX_SLOT), 4)
        .expect("static address range");
    let Some(bytes) = rt.memory().read(range) else {
        return;
    };
    let mutex_id = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
    if mutex_id == 0 {
        return;
    }
    debug_assert!(
        rt.lv2_host().mutexes().lookup(mutex_id).is_some(),
        "lv2 host handoff witness: liblv2's once-mutex slot at 0x{:016x} references \
         mutex id 0x{:08x} but the host has no such entry",
        LIBLV2_ONCE_MUTEX_SLOT,
        mutex_id,
    );
}

#[cfg(test)]
#[path = "tests/boot_tests.rs"]
mod tests;
