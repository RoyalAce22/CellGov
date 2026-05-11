//! Boot preparation shared between `run-game` and `bench-boot`.
//!
//! Both step loops see a byte-identical setup.

use std::time::{Duration, Instant};

use cellgov_core::{default_budget_for_mode, Runtime, RuntimeMode};
use cellgov_ppu::prx::HleBinding;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

use cellgov_ps3_abi::process_address_space::{
    PS3_CHILD_STACKS_BASE, PS3_CHILD_STACKS_SIZE, PS3_PRIMARY_STACK_BASE, PS3_PRIMARY_STACK_SIZE,
    PS3_PRIMARY_STACK_TOP, PS3_RSX_BASE, PS3_RSX_SIZE, PS3_SPU_RESERVED_BASE,
    PS3_SPU_RESERVED_SIZE,
};

use super::manifest::TitleManifest;
use super::prx::{build_nid_map, load_firmware_prx, pre_init_tls, run_module_start, PrxLoadInfo};
use crate::cli::exit::{die, load_file_or_die};

#[allow(dead_code)]
pub(super) struct PreparedBoot {
    pub rt: Runtime,
    pub hle_bindings: Vec<HleBinding>,
    pub elf_data: Vec<u8>,
    pub mem_size: usize,
    pub load_entry_opd: u64,
    pub proc_param: Option<cellgov_ppu::loader::SysProcessParam>,
    pub prx_info: Option<PrxLoadInfo>,
    pub timings: StartupTimings,
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
    pub strict_reserved: bool,
    pub dump_at_pc: Option<u64>,
    pub dump_skip: u32,
    /// Cap for `run_module_start`; independent of the caller's step-loop cap.
    pub module_start_max_steps: usize,
    pub print_banner: bool,
    pub profile_pairs: bool,
    /// Echoed in the banner; the value the caller will pass to `Runtime::new`.
    pub runtime_max_steps: usize,
    /// Applied after `module_start`, before `Runtime` construction.
    pub patch_bytes: &'a [(u64, u8)],
    /// Guest addresses to dump 32 bytes at after patches.
    pub dump_mem_boot_addrs: &'a [u64],
    pub budget_override: Option<u64>,
}

pub(super) fn prepare(opts: PrepareOptions<'_>) -> PreparedBoot {
    let t_start = Instant::now();
    let elf_data = load_file_or_die(opts.elf_path);

    let required_size = cellgov_ppu::loader::required_memory_size(&elf_data)
        .unwrap_or_else(|e| die(&format!("failed to parse ELF: {e:?}")));

    // Main region spans the user-memory region (0x00010000+) and the EBOOT
    // load region (0x10000000+) as one contiguous backing; 64KB alignment
    // plus 2 MiB headroom for PRX.
    let min_for_kernel = 0x4000_0000usize;
    let game_size = ((required_size + 0xFFFF) & !0xFFFF) + 0x200000;
    let mem_size = game_size.max(min_for_kernel);
    let mut state = cellgov_ppu::state::PpuState::new();
    let reserved_access = if opts.strict_reserved {
        cellgov_mem::RegionAccess::ReservedStrict
    } else {
        cellgov_mem::RegionAccess::ReservedZeroReadable
    };
    // `[rsx] mirror = true` needs the region writable so the PPU's
    // put-pointer store lands in memory for the writeback mirror to
    // route into the runtime cursor. `--strict-reserved` overrides.
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

    // Callback-return trampoline must live inside the base-0 region: the
    // PPU's instruction-fetch path only reads from region 0, so the body
    // sits in the pre-user-heap scratch zone (`0..0x10000`) rather than
    // the natural high-memory address.
    {
        use cellgov_ps3_abi::callback_dispatch::{
            CALLBACK_RETURN_CODE_ADDR, CALLBACK_RETURN_OPD_ADDR, TRAMPOLINE_CODE_BYTES,
            TRAMPOLINE_OPD_BYTES,
        };
        mem.apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new(CALLBACK_RETURN_CODE_ADDR as u64),
                TRAMPOLINE_CODE_BYTES.len() as u64,
            )
            .expect("callback trampoline code range must be valid"),
            &TRAMPOLINE_CODE_BYTES,
        )
        .expect("callback trampoline code commit");
        mem.apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new(CALLBACK_RETURN_OPD_ADDR as u64),
                TRAMPOLINE_OPD_BYTES.len() as u64,
            )
            .expect("callback trampoline OPD range must be valid"),
            &TRAMPOLINE_OPD_BYTES,
        )
        .expect("callback trampoline OPD commit");
    }
    let t_mem_alloc = t_start.elapsed();

    let load_result = cellgov_ppu::loader::load_ppu_elf(&elf_data, &mut mem, &mut state)
        .unwrap_or_else(|e| die(&format!("failed to load ELF: {e:?}")));
    let t_elf_load = t_start.elapsed();

    let tramp_base = ((required_size + 0xFFF) & !0xFFF) as u32;
    // 256 OPDs (stride 8 = 0x800) + 256 bodies (stride 16 = 0x1000).
    const HLE_PS3_SPEC_EXTENT: u64 = 0x1800;
    let opd_override: Option<u32> = match std::env::var("CELLGOV_HLE_OPD_BASE") {
        Ok(s) => {
            let v =
                u32::from_str_radix(s.trim().trim_start_matches("0x"), 16).unwrap_or_else(|e| {
                    die(&format!("CELLGOV_HLE_OPD_BASE={s:?}: not a hex u32 ({e})"))
                });
            if v < tramp_base {
                die(&format!(
                    "CELLGOV_HLE_OPD_BASE=0x{v:x} overlaps ELF load region (must be >= 0x{tramp_base:x})"
                ));
            }
            let end = v as u64 + HLE_PS3_SPEC_EXTENT;
            if end > mem_size as u64 {
                die(&format!(
                    "CELLGOV_HLE_OPD_BASE=0x{v:x}: extent 0x{end:x} exceeds mem_size 0x{:x}",
                    mem_size
                ));
            }
            Some(v)
        }
        Err(_) => None,
    };
    let hle_layout = match opd_override {
        Some(opd_base) => cellgov_ppu::prx::HleLayout::Ps3Spec {
            opd_base,
            body_base: opd_base + 256 * 8,
        },
        None => cellgov_ppu::prx::HleLayout::Legacy24,
    };
    let hle_bindings = match cellgov_ppu::prx::parse_imports(&elf_data) {
        Ok(modules) => {
            let bindings = cellgov_ppu::prx::bind_hle_stubs_with_layout(
                &modules, &mut mem, hle_layout, tramp_base,
            );
            if opts.print_banner {
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
            }
            bindings
        }
        Err(e) => {
            eprintln!(
                "imports: HLE parse failed: {e:?}; guest will crash on the first unresolved stub"
            );
            vec![]
        }
    };
    let t_hle_bind = t_start.elapsed();

    let prx_info = load_firmware_prx(opts.firmware_dir, &hle_bindings, &mut mem, tramp_base);
    let t_prx_load = t_start.elapsed();
    if opts.firmware_dir.is_some() && prx_info.is_none() {
        eprintln!(
            "prx: firmware directory was supplied but no PRX was loaded; HLE-only bindings in use"
        );
    }

    pre_init_tls(&elf_data, &mut mem);

    // Unset and empty both mean "do not skip".
    let skip_ms = match std::env::var("CELLGOV_SKIP_MODULE_START") {
        Ok(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => true,
            "0" | "false" | "no" | "off" | "" => false,
            other => die(&format!(
                "CELLGOV_SKIP_MODULE_START={other:?}: expected 0/1/true/false/yes/no/on/off"
            )),
        },
        Err(_) => false,
    };
    match (&prx_info, skip_ms) {
        (Some(info), false) => {
            mem = run_module_start(mem, info, &hle_bindings, opts.module_start_max_steps);
        }
        (Some(_), true) => {
            // stderr keeps the banner pure-stdout across run-game and bench-boot.
            eprintln!("module_start: skipped (CELLGOV_SKIP_MODULE_START set)");
        }
        (None, true) => {
            eprintln!(
                "module_start: CELLGOV_SKIP_MODULE_START set, but no PRX was loaded -- flag has no effect"
            );
        }
        (None, false) => {}
    }

    state.gpr[1] = PS3_PRIMARY_STACK_TOP;
    state.lr = 0;

    let user_region_end = super::observation::elf_user_region_end(&elf_data);
    let trampoline_area_end = match opd_override {
        Some(opd_base) => (opd_base as usize) + HLE_PS3_SPEC_EXTENT as usize,
        None => {
            // Legacy24: one 24-byte stub per binding at tramp_base.
            // alloc_floor must clear this span or user-memory allocations
            // overwrite HLE stubs.
            let end = (tramp_base as usize) + hle_bindings.len() * 24;
            (end + 0xFFFF) & !0xFFFF
        }
    };
    let alloc_floor = user_region_end.max(trampoline_area_end);
    let alloc_base = ((alloc_floor + 0xFFFF) & !0xFFFF).max(0x0001_0000) as u32;

    let tls_info = cellgov_ppu::loader::find_tls_segment(&elf_data);
    let tls_template = cellgov_ppu::loader::extract_tls_template_bytes(&elf_data);
    match (tls_info.is_some(), tls_template.is_some()) {
        (false, true) => eprintln!(
            "tls: PT_TLS bytes extractable but find_tls_segment returned None; GPRs 8/9/10 will be zero"
        ),
        (true, false) => eprintln!(
            "tls: find_tls_segment found PT_TLS but extract_tls_template_bytes returned None; no TLS template installed, child-thread TLS will be uninitialized"
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

    // max_steps is divided by the budget so the user-visible cap stays in
    // instruction units regardless of batch size.
    let mode = RuntimeMode::FaultDriven;
    let step_budget: u64 = opts
        .budget_override
        .unwrap_or_else(|| default_budget_for_mode(mode).raw())
        .max(1);
    // 32-bit hosts: `as usize` truncates a u64 multiple of 2^32 to 0.
    let step_budget_usize = (step_budget as usize).max(1);
    let adjusted_max_steps = (opts.runtime_max_steps / step_budget_usize).max(1);

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
        if let Some(ref info) = prx_info {
            println!(
                "prx: {} at 0x{:x} (toc=0x{:x}, {} relocs, {}/{} imports resolved)",
                info.name,
                info.base,
                info.toc,
                info.relocs_applied,
                info.resolved,
                info.total_imports,
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

    for &(addr, val) in opts.patch_bytes {
        match cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr), 1) {
            Some(range) => match mem.apply_commit(range, &[val]) {
                Ok(()) => {
                    if opts.print_banner {
                        println!("patch: byte 0x{addr:x} = 0x{val:02x}");
                    }
                }
                Err(e) => eprintln!(
                    "patch: byte 0x{addr:x} = 0x{val:02x} FAILED ({e:?}); target not committed"
                ),
            },
            None => eprintln!("patch: byte 0x{addr:x}: invalid address range, skipped"),
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

    // Code outside region 0 (PRX bodies above 0x10000000) falls through
    // to decode-on-fetch; Runtime stepping invalidates stale slots via
    // `invalidate_code` on each committed SharedWriteIntent.
    let shadow = cellgov_ppu::shadow::PredecodedShadow::build(0, mem.as_bytes());

    let mut rt = Runtime::new(mem, Budget::new(step_budget), adjusted_max_steps);
    rt.set_mode(mode);
    rt.set_hle_heap_base(0x10410000);
    rt.set_hle_nids(build_nid_map(&hle_bindings));
    rt.lv2_host_mut().set_mem_alloc_base(alloc_base);
    if let Some((bytes, memsz, align, vaddr)) = tls_template {
        rt.lv2_host_mut()
            .set_tls_template(cellgov_lv2::TlsTemplate::new(bytes, memsz, align, vaddr));
    }
    let primary_unit_id = rt.registry_mut().register_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = state;
        unit.set_instruction_shadow(shadow);
        if let Some(pc) = opts.dump_at_pc {
            unit.set_break_pc(pc, opts.dump_skip);
        }
        if opts.profile_pairs {
            unit.set_profile_mode(true);
        }
        unit
    });
    // Sync primitives (lwmutex, mutex, semaphore, event queue, cond)
    // resolve the caller's PpuThreadId from its UnitId via this table;
    // without the seed entry the primary's sync calls fall back to ESRCH.
    rt.lv2_host_mut().seed_primary_ppu_thread(
        primary_unit_id,
        cellgov_lv2::PpuThreadAttrs {
            entry: load_result.entry,
            arg: 0,
            stack_base: PS3_PRIMARY_STACK_BASE as u32,
            stack_size: PS3_PRIMARY_STACK_SIZE as u32,
            priority: 1001,
            tls_base: tls_info.map(|t| t.vaddr as u32).unwrap_or(0),
        },
    );

    // Child threads have no predecoded shadow and fall through to
    // decode-on-fetch.
    rt.set_ppu_factory(|id, init| {
        let mut unit = PpuExecutionUnit::new(id);
        {
            let state = unit.state_mut();
            state.pc = init.entry_code;
            state.gpr[1] = init.stack_top;
            state.gpr[2] = init.entry_toc;
            state.gpr[3] = init.arg;
            // extra_args populates r4..=r10 for callback-dispatch workers
            // carrying the parent's r3..=r10 capture; zero on the
            // sys_ppu_thread_create path.
            for (i, value) in init.extra_args.iter().enumerate() {
                state.gpr[4 + i] = *value;
            }
            state.gpr[13] = init.tls_base;
            state.lr = init.lr_sentinel;
        }
        Box::new(unit)
    });

    // Cell BE convention: args 0..3 map to r3..r6 (arg0 -> r3, etc.).
    rt.set_spu_factory(|id, init| {
        use cellgov_spu::{loader as spu_loader, SpuExecutionUnit};
        let mut unit = SpuExecutionUnit::new(id);
        spu_loader::load_spu_elf(&init.ls_bytes, unit.state_mut()).unwrap();
        unit.state_mut().pc = init.entry_pc;
        unit.state_mut().set_reg_word_splat(1, init.stack_ptr);
        unit.state_mut().set_reg_word_splat(3, init.args[0] as u32);
        unit.state_mut().set_reg_word_splat(4, init.args[1] as u32);
        unit.state_mut().set_reg_word_splat(5, init.args[2] as u32);
        unit.state_mut().set_reg_word_splat(6, init.args[3] as u32);
        Box::new(unit)
    });

    // Resolve `sysSpuImageOpen("/app_home/spu_main.elf")` against a
    // sibling of the EBOOT.
    if let Some(parent) = std::path::Path::new(opts.elf_path).parent() {
        let spu_candidate = parent.join("spu_main.elf");
        if spu_candidate.exists() {
            match std::fs::read(&spu_candidate) {
                Ok(bytes) => {
                    rt.lv2_host_mut()
                        .content_store_mut()
                        .register(b"/app_home/spu_main.elf", bytes);
                }
                Err(e) => {
                    eprintln!(
                        "run-game: WARN: cannot read {}: {}",
                        spu_candidate.display(),
                        e
                    );
                }
            }
        }
    }

    // Registration runs before any title-side step so the first
    // sys_fs_open already sees the blobs. Missing host files surface as
    // a startup error rather than a runtime FS fault.
    //
    // Resolution priority (high to low):
    //   1. override env var (gitignored developer-local content)
    //   2. EBOOT-relative USRDIR auto-discovery
    //   3. manifest's checked-in base (synthetic stubs)
    if let Some(content) = opts.title.content.as_ref() {
        let workspace_root = std::env::current_dir()
            .unwrap_or_else(|e| die(&format!("cannot read CWD for content base resolution: {e}")));
        let override_base =
            super::content::override_base_from_env(content, |name| std::env::var(name).ok());
        // EBOOT path resolves under USRDIR/<title>/EBOOT.elf; its parent
        // is the title's USRDIR. Manifest host paths are USRDIR-relative
        // (e.g. `Data/Resources/first.xml`).
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

    // Mount-table registration runs after content blobs so the
    // dispatch layer sees them in the same order they appear in the
    // manifest, and so a mount can shadow nothing in FsStore that
    // [content] just registered (the FsStore path-existence check
    // wins over mount resolution).
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
        hle_bindings,
        elf_data,
        mem_size,
        load_entry_opd: load_result.entry,
        proc_param,
        prx_info,
        timings: StartupTimings {
            mem_alloc: t_mem_alloc,
            elf_load: t_elf_load - t_mem_alloc,
            hle_bind: t_hle_bind - t_elf_load,
            prx_load: t_prx_load - t_hle_bind,
        },
    }
}
