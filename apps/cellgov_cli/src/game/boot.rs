//! Boot preparation shared between `run-game` and `bench-boot`.

use std::time::{Duration, Instant};

use cellgov_core::{default_budget_for_mode, Runtime, RuntimeMode};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

use cellgov_ps3_abi::process_address_space::{
    PS3_CHILD_STACKS_BASE, PS3_CHILD_STACKS_SIZE, PS3_PRIMARY_STACK_BASE, PS3_PRIMARY_STACK_SIZE,
    PS3_PRIMARY_STACK_TOP, PS3_RSX_BASE, PS3_RSX_SIZE, PS3_SPU_RESERVED_BASE,
    PS3_SPU_RESERVED_SIZE,
};

use super::manifest::TitleManifest;
use super::prx::{load_firmware_prx, load_firmware_set_bound, pre_init_tls, run_module_start};
use crate::cli::env::parse_env_bool;
use crate::cli::exit::{die, load_ppu_image_or_die};

/// Default primary-thread priority when the title's `sys_proc_param`
/// block is absent.
const DEFAULT_PRIMARY_PRIO: u32 = 1001;

/// Bump-arena base for HLE-side allocations, above the TLS scratch
/// at `0x10400000`.
pub const HLE_HEAP_BASE: u32 = 0x10410000;

/// Narrow `u64` to `u32` at the host/guest boundary; dies on
/// overflow with `label` named.
fn u32_or_die(label: &str, value: u64) -> u32 {
    u32::try_from(value)
        .unwrap_or_else(|_| die(&format!("{label}: 0x{value:x} does not fit in u32")))
}

/// Which firmware-loading pipeline `boot::prepare` should drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BootMode {
    SinglePrx,
    FirmwareSet,
}

impl BootMode {
    pub(crate) fn as_cli_str(self) -> &'static str {
        match self {
            BootMode::SinglePrx => "single-prx",
            BootMode::FirmwareSet => "firmware-set",
        }
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
    pub firmware_dir: Option<&'a str>,
    pub boot_mode: BootMode,
    pub strict_reserved: bool,
    pub dump_at_pc: Option<u64>,
    pub dump_skip: u32,
    /// Cap for `run_module_start`; independent of the caller's step-loop cap.
    pub module_start_max_steps: usize,
    pub print_banner: bool,
    pub profile_pairs: bool,
    pub runtime_max_steps: usize,
    /// Applied after `module_start`, before `Runtime` construction.
    pub patch_bytes: &'a [(u64, u8)],
    pub dump_mem_boot_addrs: &'a [u64],
    pub budget_override: Option<Budget>,
    /// When true, switch runtime mode from `FaultDriven` to
    /// `DeterminismCheck` so per-step `PpuStateHash` records land in
    /// the runtime's trace buffer. The caller writes the buffer to
    /// disk; `prepare` only flips the mode.
    pub capture_state_trace: bool,
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
    let elf_data = load_ppu_image_or_die(opts.elf_path);

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
    let mut mem = cellgov_mem::GuestMemory::from_regions(vec![
        cellgov_mem::Region::new(0, mem_size, "main", cellgov_mem::PageSize::Page64K),
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

    let tramp_base = {
        let rounded = required_size.checked_add(0xFFF).unwrap_or_else(|| {
            die(&format!(
                "required_size=0x{required_size:x} + 0xFFF overflows usize"
            ))
        }) & !0xFFF;
        u32_or_die("tramp_base", rounded as u64)
    };

    // Parse imports so the firmware PRX loader can resolve game-side
    // OPD stubs to firmware exports.
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

    // Code floor used to clear the now-deleted HLE trampoline region;
    // with the binder gone, only the ELF PT_LOAD ranges and firmware
    // PRX load region matter.
    let code_floor = tramp_base;

    let prx_modules = match opts.boot_mode {
        BootMode::SinglePrx => {
            match load_firmware_prx(opts.firmware_dir, &modules, &mut mem, code_floor) {
                Some(info) => vec![info],
                None => Vec::new(),
            }
        }
        BootMode::FirmwareSet => {
            load_firmware_set_bound(opts.firmware_dir, &modules, &mut mem, code_floor)
        }
    };
    let t_prx_load = t_start.elapsed();
    if opts.firmware_dir.is_some() && prx_modules.is_empty() {
        eprintln!("prx: firmware directory was supplied but no PRX was loaded");
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

    let skip_ms = parse_env_bool("CELLGOV_SKIP_MODULE_START");
    match (prx_modules.is_empty(), skip_ms) {
        (false, false) => {
            for info in &prx_modules {
                mem = run_module_start(mem, info, opts.module_start_max_steps, alloc_base);
            }
        }
        (false, true) => {
            eprintln!("module_start: skipped (CELLGOV_SKIP_MODULE_START set)");
        }
        (true, true) => {
            eprintln!(
                "module_start: CELLGOV_SKIP_MODULE_START set, but no PRX was loaded -- flag has no effect"
            );
        }
        (true, false) => {}
    }

    state.gpr[1] = PS3_PRIMARY_STACK_TOP;
    state.lr = 0;

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
    state.gpr[3] = 0;
    state.gpr[4] = 0;
    state.gpr[5] = 0;
    state.gpr[6] = 0;
    state.gpr[7] = 0x0100_0000;
    state.gpr[8] = tls_info.map(|t| t.vaddr).unwrap_or(0);
    state.gpr[9] = tls_info.map(|t| t.filesz).unwrap_or(0);
    state.gpr[10] = tls_info.map(|t| t.memsz).unwrap_or(0);
    state.gpr[11] = load_result.entry;
    let proc_param = cellgov_ppu::loader::find_sys_process_param(&elf_data);
    let malloc_pagesize = proc_param.map(|p| p.malloc_pagesize).unwrap_or(0x100000);
    state.gpr[12] = malloc_pagesize as u64;

    let mode = if opts.capture_state_trace {
        RuntimeMode::DeterminismCheck
    } else {
        RuntimeMode::FaultDriven
    };
    let step_budget = {
        let b = opts
            .budget_override
            .unwrap_or_else(|| default_budget_for_mode(mode));
        // Floor at 1: a zero budget exits the step loop before any work.
        if b.is_exhausted() {
            Budget::new(1)
        } else {
            b
        }
    };
    let step_budget_usize = (step_budget.raw() as usize).max(1);
    // Refuse silent floor-to-1: a max_steps lower than the budget runs
    // up to `budget` instructions, not the user's chosen cap.
    if opts.runtime_max_steps < step_budget_usize {
        die(&format!(
            "max_steps={} below budget={step_budget}; raise --max-steps or lower --budget",
            opts.runtime_max_steps
        ));
    }
    let adjusted_max_steps = opts.runtime_max_steps / step_budget_usize;

    // Primary-thread attributes from sys_proc_param: silent fallback
    // values diverge from RPCS3's reading of the same block.
    let primary_prio: u32 = match proc_param.map(|p| p.primary_prio) {
        Some(p) => {
            u32::try_from(p).unwrap_or_else(|_| die(&format!("primary_prio={p} is negative")))
        }
        None => DEFAULT_PRIMARY_PRIO,
    };
    // `primary_stacksize == 0` reads as "use kernel
    // default", so it falls through to PS3_PRIMARY_STACK_SIZE rather
    // than seeding a zero-sized stack that faults on the first frame.
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

    // Determinism contract: a patch that fails on this host produces a
    // boot the user did not request. Surface every failure as a hard
    // exit instead of letting two hosts diverge silently.
    for &(addr, val) in opts.patch_bytes {
        let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 1)
            .unwrap_or_else(|| die(&format!("patch: byte 0x{addr:x}: invalid address range")));
        mem.apply_commit(range, &[val]).unwrap_or_else(|e| {
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
            Some(r) => match mem.read(r) {
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
                None => println!("mem[0x{addr:x}]: unmapped"),
            },
        }
    }

    // Bound the predecode shadow to the upper end of executable code
    // (`alloc_floor` = max of PT_LOAD vaddr end, HLE trampoline/OPD
    // end, and PRX placement end). Anything above this address is
    // either uncommitted zeros or non-executable data; the shadow
    // would decode every 32-bit word as an instruction anyway and
    // pay O(region size). The runtime fetch path's `None` branch
    // falls back to live decode + `refresh`, so an out-of-bounds PC
    // is still handled correctly -- it just doesn't cache.
    let shadow_extent = (alloc_floor as usize).min(mem.as_bytes().len());
    let t_shadow_start = std::time::Instant::now();
    let shadow = cellgov_ppu::shadow::PredecodedShadow::build(0, &mem.as_bytes()[..shadow_extent]);
    if parse_env_bool("CELLGOV_RUNGAME_PROFILE") {
        eprintln!(
            "rungame_profile_shadow: PredecodedShadow::build over {shadow_extent} bytes took {:.2}ms \
             (alloc_floor=0x{alloc_floor:08x} user_region_end=0x{user_region_end:08x} \
             code_floor=0x{code_floor:08x} prx_region_end=0x{prx_region_end:08x})",
            t_shadow_start.elapsed().as_secs_f64() * 1000.0
        );
    }

    let mut rt = Runtime::new(mem, step_budget, adjusted_max_steps);
    rt.set_mode(mode);
    rt.lv2_host_mut().set_mem_alloc_base(alloc_base);
    // Cross-module contract: firmware-side `_sys_prx_load_module(path)`
    // resolves the guest path against this registry to recover the
    // kernel id; an empty stem leaves the module unreachable from
    // libsysmodule's load worker, which usually means a corrupted
    // header upstream.
    for info in &prx_modules {
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
    // Without this seed entry the primary's sync calls fall back to ESRCH:
    // sync primitives resolve PpuThreadId from UnitId via this table.
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
    //
    // # Panics
    //
    // The .expect on load_spu_elf below covers a structural gap: this
    // closure runs at SPU-thread-create time, not at sys_spu_image_open
    // time, so a malformed title-provided ELF first surfaces here as a
    // panic. The clean fix is upstream pre-validation at image-open
    // time so the bytes that reach this closure are already vetted;
    // until then, the panic is the failure mode for a malformed title.
    // Cleaning this up requires changing the SpuFactory trait signature
    // to return Result.
    rt.set_spu_factory(|id, init| {
        use cellgov_spu::{loader as spu_loader, SpuExecutionUnit};
        let mut unit = SpuExecutionUnit::new(id);
        spu_loader::load_spu_elf(&init.ls_bytes, unit.state_mut())
            .expect("game boot: load_spu_elf on title-provided ELF; failure indicates a bad LV2 thread init");
        unit.state_mut().pc = init.entry_pc;
        unit.state_mut().set_reg_word_splat(1, init.stack_ptr);
        // SPU preferred-slot args are 32-bit per Cell BE; the `as u32`
        // narrow is correct.
        unit.state_mut().set_reg_word_splat(3, init.args[0] as u32);
        unit.state_mut().set_reg_word_splat(4, init.args[1] as u32);
        unit.state_mut().set_reg_word_splat(5, init.args[2] as u32);
        unit.state_mut().set_reg_word_splat(6, init.args[3] as u32);
        Box::new(unit)
    });

    // Resolve `sysSpuImageOpen("/app_home/spu_main.elf")` against an
    // EBOOT sibling.
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
                    let label = match source {
                        super::content::ContentBaseSource::Manifest => {
                            format!("manifest base ({})", content.base)
                        }
                        super::content::ContentBaseSource::Usrdir { path } => {
                            format!("EBOOT-adjacent USRDIR ({})", path.display())
                        }
                        super::content::ContentBaseSource::Override { env } => {
                            format!(
                                "override env {env}={}",
                                override_base
                                    .as_ref()
                                    .map(|p| p.display().to_string())
                                    .unwrap_or_default(),
                            )
                        }
                    };
                    println!(
                        "content: registered {} blob(s) from {label}",
                        content.files.len(),
                    );
                }
            }
            Err(e) => die(&format!("content provider failed: {e}")),
        }
    }

    // Mount registration follows content so the FsStore path-existence
    // check wins over mount resolution for blobs registered above.
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
