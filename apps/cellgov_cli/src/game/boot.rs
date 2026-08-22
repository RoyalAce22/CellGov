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

/// Decode a `sys_proc_param.primary_stacksize` declaration to bytes.
///
/// The field carries either a raw byte count or a kernel sentinel;
/// mapping per RPCS3 `sys_process.h`
/// (`SYS_PROCESS_PRIMARY_STACK_SIZE_*`) and `PPUModule.cpp`
/// `ppu_load_exec`.
fn decode_primary_stacksize(declared: u32) -> u32 {
    match declared {
        0x10 => 32 * 1024,
        0x20 => 64 * 1024,
        0x30 => 96 * 1024,
        0x40 => 128 * 1024,
        0x50 => 256 * 1024,
        0x60 => 512 * 1024,
        0x70 => 1024 * 1024,
        raw => raw,
    }
}

/// Seeded ring size per slot: the dispatcher's six non-zero field
/// budgets (56+8+76+4+22+10 = 176 bytes) drain inside it, so the
/// ring never depletes mid-record.
const CELLSYSUTIL_RING_LIMIT: u32 = 256;

/// Boot-state seed for the cellSysutil slot-state shm.
///
/// Models the first producer record an external firmware producer
/// would have delivered before the title ran. Field consumers are
/// the libsysutil wait-fn guard reads: state@+20 (`!= 2` falls
/// through), cursor@+16 vs limit@+4 (`<` enters the drain),
/// read_pos@+8 / write_pos@+12 / data_offset@+0 drive the
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
    /// Where the served program-authority-id came from: `"self"`
    /// (SELF identification header), `"fallback"` (raw-ELF retail
    /// fallback), or `"forced"` (the adversarial env knob). Lets the
    /// authority witness distinguish a real per-title id from a
    /// parse regression that fell back to the shared retail constant.
    pub authid_source: &'static str,
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
    /// Program authority id from the SELF identification header;
    /// `None` (raw-ELF input) keeps the host's retail fallback.
    pub authority_id: Option<u64>,
    /// `ctrl_flags1` from the SELF's plaintext capability header;
    /// `None` for raw-ELF input and for a SELF without the record.
    pub control_flags1: Option<u32>,
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
    /// Guest argv for the primary thread, `argv[0]` included. Empty
    /// keeps the no-args entry state (r3..r6 = 0).
    pub guest_args: &'a [String],
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

    let (mut prx_modules, verified_firmware, mut host_link) = load_firmware_set_bound(
        opts.firmware_dir,
        &modules,
        &mut mem,
        code_floor,
        matches!(
            opts.title.source,
            crate::game::manifest::GameSource::FirmwareExec { .. }
        ),
    );
    let t_prx_load = t_start.elapsed();
    if prx_modules.is_empty() {
        // No firmware loaded: install trampolines so calls through
        // unresolved imports produce a structured fault.
        let (info, requesters) =
            super::prx::install_unresolved_trampolines_only(&modules, &mut mem, code_floor as u64);
        if let Some(info) = info {
            prx_modules.push(info);
        }
        host_link.unresolved_requesters = requesters;
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
            // A zero budget stalls the runtime without retiring work
            // (`cellgov_core::Runtime::new` "Zero values"), so the boot
            // would never advance. Raise it, but name the coercion so
            // the run is not read as honouring what was asked for.
            eprintln!("boot: budget 0 retires no work; raised to 1");
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
    let (adjusted_max_steps, effective_max_steps) =
        step_call_cap(opts.runtime_max_steps, step_budget_usize);
    if effective_max_steps != opts.runtime_max_steps {
        eprintln!(
            "boot: max_steps={} is not a multiple of budget={step_budget}; \
             the effective cap is {effective_max_steps} retired instructions",
            opts.runtime_max_steps,
        );
    }

    let primary_prio: u32 = match proc_param.map(|p| p.primary_prio) {
        Some(p) => {
            u32::try_from(p).unwrap_or_else(|_| die(&format!("primary_prio={p} is negative")))
        }
        None => DEFAULT_PRIMARY_PRIO,
    };
    // `primary_stacksize == 0` reads as "use kernel default".
    let primary_stack_size: u32 = match proc_param.map(|p| decode_primary_stacksize(p.primary_stacksize)) {
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
    // Bind the manifest-verified PUP identity so the boot's state hash
    // is a function of which firmware revision fed it.
    if let Some(fw) = &verified_firmware {
        rt.lv2_host_mut()
            .set_firmware_identity(&fw.image_version, fw.pup_sha256);
    }
    // Plumb the title's recorded SDK version into the LV2 host so
    // `sys_process_get_sdk_version` reports the value cellSysutil's
    // SDK-keyed init dispatcher gates on. An absent param segment
    // leaves the PSL1GHT homebrew sentinel in place.
    if let Some(p) = proc_param {
        rt.lv2_host_mut().set_sdk_version(p.sdk_version);
    }
    // Pre-game system state: the cellSysutil slot-state shm arrives
    // seeded with one producer record per slot, applied when the
    // keyed shm is first mapped (sc 337).
    rt.lv2_host_mut()
        .register_system_seed(cellsysutil_system_seed());
    // Boot identity served by sys_ss_access_control_engine pkg 2.
    // Firmware modules classify callers by this value; libsysmodule's
    // module_start runs full init only for non-system authids.
    if let Some(authid) = opts.authority_id {
        rt.lv2_host_mut().set_program_authority_id(authid);
    }
    // Adversarial knob for the authority-id tripwire test: forcing
    // the bdj.self system authid makes the cellSysmodule
    // LoadModule-failure signature reappear.
    let (authid_label, authid_source) = if parse_env_bool("CELLGOV_FORCE_SYSTEM_AUTHID") {
        rt.lv2_host_mut()
            .set_program_authority_id(cellgov_ps3_abi::sce::BDJ_SELF_PROGRAM_AUTHORITY_ID);
        (
            "forced system authid (CELLGOV_FORCE_SYSTEM_AUTHID)",
            "forced",
        )
    } else if opts.authority_id.is_some() {
        ("from SELF identification header", "self")
    } else {
        ("raw-ELF input -- retail-application fallback", "fallback")
    };
    println!(
        "program_authority_id: 0x{:016x} ({})",
        rt.lv2_host().program_authority_id(),
        authid_label,
    );
    // Process privilege, from the SELF's plaintext capability header.
    if let Some(flags) = opts.control_flags1 {
        rt.lv2_host_mut().set_control_flags1(flags);
    }
    // Resolution source for the sc 484 CoreOS manual import link, and
    // the requester map behind the unresolved-import diagnostic.
    rt.lv2_host_mut().set_firmware_exports(host_link.exports);
    rt.lv2_host_mut()
        .set_unresolved_import_requesters(host_link.unresolved_requesters);
    {
        let h = rt.lv2_host();
        println!(
            "ctrl_flags1: 0x{:08x} (root={} debug_or_root={} debug={} coreos={})",
            h.control_flags1(),
            h.has_root_perm(),
            h.debug_or_root(),
            h.has_debug_perm(),
            h.is_coreos(),
        );
    }
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
        let id = rt.lv2_host_mut().prx_registry_mut().register(
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
        // Boot runs every firmware module's module_start below, so
        // these enter the resident/started state LV2 refuses to
        // unload; only sc 480 miss stubs stay unstarted.
        rt.lv2_host_mut().prx_registry_mut().mark_started(id);
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
            state.set_gpr(1, init.stack_top);
            state.set_gpr(2, init.entry_toc);
            state.set_gpr(3, init.arg);
            for (i, value) in init.extra_args.iter().enumerate() {
                state.set_gpr(4 + i, *value);
            }
            state.set_gpr(13, init.tls_base);
            state.set_lr(init.lr_sentinel);
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

    // Spawned-child image loads (`_sys_process_spawn` /
    // `sys_process_spawns_a_self2`): install a self-contained child
    // space and load the ELF the same way the microtest harness
    // does. SELF paths resolve through the LV2 content store.
    rt.set_process_spawn_loader(|elf_bytes, mem| {
        // A child image may arrive SCE-wrapped (vsh spawns SELFs, not
        // raw ELFs). The spawn loader is APP-keyed: klicensee
        // resolution belongs to the title-install layer, which is not
        // reachable from inside the runtime.
        let plaintext = cellgov_install::self_image::to_plaintext_elf(
            elf_bytes,
            cellgov_install::self_image::KeyPolicy::AppOnly,
        )
        .map_err(|e| cellgov_core::ProcessSpawnLoadError::ImageParse {
            detail: format!("child SELF: {e}"),
        })?;
        let elf_bytes: &[u8] = &plaintext;
        let required = cellgov_ppu::loader::required_memory_size(elf_bytes).map_err(|e| {
            cellgov_core::ProcessSpawnLoadError::ImageParse {
                detail: e.to_string(),
            }
        })?;
        let child_mem_size = spawned_child_region_size(required)?;
        mem.install_region(0, child_mem_size, "spawned", cellgov_mem::PageSize::Page64K)
            .map_err(|source| cellgov_core::ProcessSpawnLoadError::RegionInstall { source })?;
        let exit_stub_addr = child_exit_stub_addr(required);
        // li r11, 22; sc -- entered if the child's entry returns. r11
        // is the LV2 syscall number and 22 is `_sys_process_exit`
        // (RPCS3 `lv2.cpp` syscall table); the exit status is whatever
        // the entry left in r3.
        let stub: [u8; 8] = [0x39, 0x60, 0x00, 0x16, 0x44, 0x00, 0x00, 0x02];
        let range = cellgov_mem::ByteRange::new(
            cellgov_mem::GuestAddr::new(exit_stub_addr),
            stub.len() as u64,
        )
        .ok_or_else(|| cellgov_core::ProcessSpawnLoadError::RegionSize {
            detail: format!("exit-stub range at 0x{exit_stub_addr:x} is not addressable"),
        })?;
        mem.apply_commit(range, &stub)
            .map_err(|source| cellgov_core::ProcessSpawnLoadError::ExitStubWrite { source })?;
        let mut state = cellgov_ppu::state::PpuState::new();
        cellgov_ppu::loader::load_ppu_elf(elf_bytes, mem, &mut state).map_err(|e| {
            cellgov_core::ProcessSpawnLoadError::ImageLoad {
                detail: e.to_string(),
            }
        })?;
        Ok(cellgov_core::SpawnedProcessImage {
            entry_code: state.pc,
            entry_toc: state.gpr[2],
            stack_top: (child_mem_size as u64) - 0x1000,
            lr_sentinel: exit_stub_addr,
        })
    });

    // Resolves `sysSpuImageOpen("/app_home/spu_main.elf")` against
    // an EBOOT sibling; same discovery for a spawn microtest's
    // child SELF.
    if let Some(parent) = std::path::Path::new(opts.elf_path).parent() {
        for (sibling, guest_path) in [
            ("spu_main.elf", b"/app_home/spu_main.elf".as_slice()),
            ("child.self", b"/app_home/child.self".as_slice()),
        ] {
            let candidate = parent.join(sibling);
            if candidate.exists() {
                let bytes = std::fs::read(&candidate).unwrap_or_else(|e| {
                    die(&format!(
                        "run-game: cannot read {}: {e}",
                        candidate.display()
                    ))
                });
                rt.lv2_host_mut()
                    .content_store_mut()
                    .register(guest_path, bytes);
            }
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
    //
    // With guest args, the args block sits at the stack top and r1
    // drops below it; r3..r6 carry argc/argv/envp/envc (layout in
    // [`super::guest_args`]). Without, the entry state is the
    // historical no-args shape.
    let args_block = if opts.guest_args.is_empty() {
        None
    } else {
        let block = super::guest_args::build_args_block(
            PS3_PRIMARY_STACK_TOP,
            primary_stack_size as u64,
            opts.guest_args,
        )
        .unwrap_or_else(|e| die(&format!("--guest-arg: {e}")));
        let range = cellgov_mem::ByteRange::new(
            cellgov_mem::GuestAddr::new(block.base),
            block.bytes.len() as u64,
        )
        .unwrap_or_else(|| {
            die(&format!(
                "--guest-arg: args block 0x{:08x}+0x{:x} is not a valid range",
                block.base,
                block.bytes.len()
            ))
        });
        rt.memory_mut()
            .apply_commit(range, &block.bytes)
            .unwrap_or_else(|e| {
                die(&format!(
                    "--guest-arg: committing args block at 0x{:08x} FAILED ({e:?})",
                    block.base
                ))
            });
        if opts.print_banner {
            println!(
                "guest args: argc={} argv=0x{:08x} r1=0x{:08x}",
                block.argc, block.argv_addr, block.initial_r1
            );
        }
        Some(block)
    };
    match &args_block {
        Some(b) => {
            // r1 sits a full linkage frame below the block (RPCS3
            // `PPUModule.cpp` `ppu_load_exec`): with r1 == block
            // base, the entry's CR/LR saves at 8(r1)/16(r1) would
            // land inside the argv pointer table.
            state.set_gpr(1, b.initial_r1);
            state.set_gpr(3, b.argc);
            state.set_gpr(4, b.argv_addr);
            state.set_gpr(5, b.envp_addr);
        }
        None => {
            state.set_gpr(1, PS3_PRIMARY_STACK_TOP);
            state.set_gpr(3, 0);
            state.set_gpr(4, 0);
            state.set_gpr(5, 0);
        }
    }
    state.set_lr(0);
    state.set_gpr(6, 0);
    state.set_gpr(7, 0x0100_0000);
    state.set_gpr(8, tls_info.map(|t| t.vaddr).unwrap_or(0));
    state.set_gpr(9, tls_info.map(|t| t.filesz).unwrap_or(0));
    state.set_gpr(10, tls_info.map(|t| t.memsz).unwrap_or(0));
    state.set_gpr(11, load_result.entry);
    state.set_gpr(12, malloc_pagesize as u64);
    // r13 is the PS3 PPC64 ABI TLS pointer; LV2 seeds it at process
    // creation and sys_initialize_tls does not touch it.
    state.set_gpr(13, super::prx::TLS_BASE + 0x7030);

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
    let (modules_started, modules_faulted) = match (prx_modules.is_empty(), skip_ms) {
        (false, false) => {
            let mut completed: usize = 0;
            let mut faulted: Vec<String> = Vec::new();
            for info in &prx_modules {
                let runnable_before = rt.registry().runnable_ids().count();
                match run_module_start(&mut rt, info, kctx_opd) {
                    Ok(ModuleStartOutcome::Completed { .. })
                    | Ok(ModuleStartOutcome::HleStubbed) => completed += 1,
                    Ok(ModuleStartOutcome::Skipped) => {}
                    // A guest fault leaves the module un-started and the
                    // boot alive: the runner already tore the transient
                    // unit down (alias dropped, unit Faulted and skipped),
                    // and one broken init out of a derived load set must
                    // not brick every other module's boot. The other
                    // error kinds can leave a parked unit behind, so they
                    // stay fatal.
                    Err(super::prx::ModuleStartError::Faulted { module, .. }) => {
                        faulted.push(module);
                    }
                    Err(e) => die(&format!("{e}")),
                }
                // Each module_start either Skipped (no unit registered,
                // count unchanged) or ran a transient unit that ended
                // Faulted -- at the return sentinel (Completed) or at
                // a real fault (Faulted) -- so the count is unchanged
                // either way. The primary's blocked override holds, so
                // the count never grew during the module's sub-loop.
                debug_assert_eq!(
                    rt.registry().runnable_ids().count(),
                    runnable_before,
                    "module_start {} left a runnable unit in the registry",
                    info.name,
                );
            }
            if !faulted.is_empty() {
                eprintln!(
                    "BENCH_MODULE_START_FAULTS: count={} modules={}",
                    faulted.len(),
                    faulted.join(",")
                );
            }
            (completed, faulted.len())
        }
        (false, true) => {
            eprintln!("module_start: skipped (CELLGOV_SKIP_MODULE_START set)");
            (0, 0)
        }
        (true, true) => {
            eprintln!(
                "module_start: CELLGOV_SKIP_MODULE_START set, but no PRX was loaded -- flag has no effect"
            );
            (0, 0)
        }
        (true, false) => (0, 0),
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

    assert_gating_state_coherent_with_host(&rt, !prx_modules.is_empty());
    // Faulted starts are witnessed skips (BENCH_MODULE_START_FAULTS),
    // so completeness is completed + faulted, not completed alone.
    // CELLGOV_SKIP_MODULE_START legitimately runs nothing (its own
    // witnessed skip), so the invariant binds only when the loop ran.
    if !skip_ms {
        debug_assert_eq!(
            modules_started + modules_faulted,
            modules_total,
            "module_start: completed {modules_started} + faulted {modules_faulted} \
             of {modules_total} modules",
        );
    }

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
        authid_source,
    }
}

/// Convert a retired-instruction cap into the `rt.step()` call cap
/// [`Runtime::new`] takes, paired with the instruction cap that
/// actually results.
///
/// `Runtime::max_steps` counts `step()` calls, each granting up to
/// `budget` retired instructions, so a request that is not a multiple
/// of the budget rounds down and the remainder is unreachable.
///
/// # Panics
///
/// `budget` must be non-zero; the caller floors it at 1.
fn step_call_cap(max_instructions: usize, budget: usize) -> (usize, usize) {
    debug_assert!(budget > 0, "caller floors the budget at 1 before dividing");
    let calls = max_instructions / budget;
    (calls, calls * budget)
}

/// Address of a spawned child's exit stub: the landing site a child
/// enters when its entry returns instead of calling
/// `sys_process_exit`.
///
/// Derived from `required` (the highest PT_LOAD end) rather than
/// fixed, so no segment of the image can land on it. `load_ppu_elf`
/// writes only `[p_vaddr, p_vaddr + p_memsz)` per PT_LOAD -- never
/// the segment's `p_align` padding -- so "at or above the highest
/// segment end" is the whole disjointness argument.
///
/// The floor of `STUB_MIN_ADDR` keeps address 0 out of the answer for
/// an image whose PT_LOADs total zero bytes. RPCS3 reaches the same
/// property by allocating its equivalent return sentinel out of the
/// main area after the image's own fixed allocations
/// (`Emu/Cell/PPUModule.cpp` `ppu_initialize_modules` fills an
/// allocated fake-OPD array, and `Emu/Cell/PPUThread.cpp`
/// `ppu_thread::fast_call` points LR at one of its slots); there
/// `vm::alloc` returns 0 only to signal failure, so 0 is a null
/// sentinel and never a code site. A stub at 0 would also silently
/// convert a guest branch through a null function pointer into a
/// clean `_sys_process_exit` (LV2 syscall 22) instead of a fault.
///
/// The caller sizes the child region with [`spawned_child_region_size`],
/// which always leaves headroom above the image, so the returned
/// address is inside the region and below the initial SP.
fn child_exit_stub_addr(required: usize) -> u64 {
    /// Lowest address the stub may occupy; keeps 0 reserved as null.
    const STUB_MIN_ADDR: u64 = 16;
    // `required_memory_size` caps every segment end at the 4 GiB EA
    // ceiling, so the round-up cannot overflow.
    (required as u64).next_multiple_of(16).max(STUB_MIN_ADDR)
}

/// Sizes a spawned child's address-space region from its ELF's
/// required memory.
///
/// The floor covers a PSL1GHT child's user region plus its
/// SYS_PROCESS_PARAM segment at 0x1000_0000 (the microtest harness
/// sizing). The child's initial SP sits one page below the region
/// end and its stack grows down toward the image, so the floor is
/// kept only when at least the 0x4000 ABI stack floor (back chain +
/// register save area, the same minimum `dispatch_ppu_thread_create`
/// enforces for child-thread stacks) fits between the image end and
/// the SP; a child that would squeeze that gap -- or whose PT_LOADs
/// reach past the floor entirely -- gets the required-size-plus-
/// headroom sizing the parent boot uses (64K alignment, 128K above
/// the image) instead of a zero-depth stack or a rejected spawn.
fn spawned_child_region_size(
    required: usize,
) -> Result<usize, cellgov_core::ProcessSpawnLoadError> {
    const CHILD_MEM_FLOOR: usize = 0x1002_0000;
    const STACK_TOP_PAD: usize = 0x1000;
    const MIN_CHILD_STACK: usize = 0x4000;
    if required.saturating_add(STACK_TOP_PAD + MIN_CHILD_STACK) <= CHILD_MEM_FLOOR {
        return Ok(CHILD_MEM_FLOOR);
    }
    required
        .checked_add(0xFFFF)
        .map(|v| v & !0xFFFF)
        .and_then(|v| v.checked_add(0x2_0000))
        .ok_or_else(|| cellgov_core::ProcessSpawnLoadError::RegionSize {
            detail: format!("required_size=0x{required:x} overflows usize"),
        })
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
