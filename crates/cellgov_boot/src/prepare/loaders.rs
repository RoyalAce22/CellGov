//! The closures the runtime calls back into -- the PPU and SPU
//! execution-unit factories, and the spawned-child image loader.

use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use cellgov_core::Runtime;
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::address_space::{PS3_PRIMARY_STACK_SIZE_MAX, PS3_RSX_IOMAP_BASE};

use super::types::{BootDebugOptions, BootServices, TitleOptions};
use crate::child_init::{ChildInitPlan, ChildInitPlans};
use crate::prx::{
    install_kernel_context_opd, install_unresolved_trampolines_only, load_firmware_set_from,
    pre_init_tls, FirmwareCandidates, PrxLoadInfo, TlsArea,
};

/// Library name -> NID -> OPD of a loaded firmware set.
type ExportMap = BTreeMap<String, BTreeMap<u32, u32>>;

/// Install the factories the runtime materializes execution units from.
///
/// Both fire again for every child the guest creates, so the PPU
/// closure captures `debug_opts` and `thread_shadow`.
pub(super) fn install_unit_factories(
    rt: &mut Runtime,
    debug_opts: BootDebugOptions,
    thread_shadow: ThreadShadow,
) {
    rt.set_ppu_factory(move |id, init| {
        Box::new(ppu_thread_unit(id, &init, &debug_opts, &thread_shadow))
    });
    rt.set_spu_factory(spu_unit);
}

/// The PPU unit of a thread the guest creates, for
/// [`Runtime::set_ppu_factory`].
///
/// The unit takes a clone of the shadow that `thread_shadow` offers.
/// The clone shares the build-time decode of the primary unit's shadow.
fn ppu_thread_unit(
    id: cellgov_event::UnitId,
    init: &cellgov_lv2::PpuThreadInitState,
    debug_opts: &BootDebugOptions,
    thread_shadow: &ThreadShadow,
) -> PpuExecutionUnit {
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
    if let Some(shadow) = thread_shadow.shadow() {
        unit.set_instruction_shadow(shadow);
    }
    debug_opts.apply(&mut unit);
    unit
}

/// The boot space's predecode shadow, for the PPU threads that the
/// factory builds.
///
/// The factory builds a unit before the runtime places it in an address
/// space, so the factory does not know the space of the unit. The shadow
/// decodes the text of the boot space, and only a spawned process owns
/// another space. Thus the first spawn withdraws the shadow permanently.
/// Each thread that the factory builds after that spawn decodes on fetch.
#[derive(Clone, Default)]
pub(super) struct ThreadShadow(Rc<RefCell<ShadowOffer>>);

/// What [`ThreadShadow`] gives the next thread.
#[derive(Default)]
enum ShadowOffer {
    /// The boot did not build the shadow yet.
    #[default]
    Pending,
    /// Each thread takes a clone.
    Offered(PredecodedShadow),
    /// No thread takes a shadow, for one of these causes:
    ///
    /// - The boot built none.
    /// - The guest spawned a process.
    Withdrawn,
}

impl ThreadShadow {
    /// Offers `shadow` to each thread the factory builds from now on.
    ///
    /// A spawn before the offer keeps the shadow withdrawn.
    pub(super) fn offer(&self, shadow: Option<&PredecodedShadow>) {
        let mut offer = self.0.borrow_mut();
        if matches!(*offer, ShadowOffer::Pending) {
            *offer = shadow.map_or(ShadowOffer::Withdrawn, |shadow| {
                ShadowOffer::Offered(shadow.clone())
            });
        }
    }

    /// Withdraws the shadow from each thread the factory builds from now
    /// on.
    fn withdraw(&self) {
        *self.0.borrow_mut() = ShadowOffer::Withdrawn;
    }

    /// Returns a clone of the shadow on offer, or `None` while none is
    /// on offer.
    fn shadow(&self) -> Option<PredecodedShadow> {
        match &*self.0.borrow() {
            ShadowOffer::Offered(shadow) => Some(shadow.clone()),
            ShadowOffer::Pending | ShadowOffer::Withdrawn => None,
        }
    }
}

/// The SPU unit a thread-group image materializes as, for
/// [`Runtime::set_spu_factory`].
///
/// The image loads into local store, and the unit starts at the entry
/// PC with its stack pointer in r1. Cell BE convention: args 0..3 map
/// to r3..r6 (arg0 -> r3, etc.), each as one 64-bit value in the
/// preferred slot: word 0 holds its high half, word 1 its low half, and
/// words 2 and 3 are zero. A SPURS kernel reads the whole doubleword when
/// it adds an offset across both words of its instance address, so it
/// needs the argument there and not its low word twice. No public
/// document in the set states the layout; it is unestablished, and the
/// open toolchain's SPU programs declare their arguments as 64-bit
/// values.
///
/// # Errors
///
/// [`cellgov_core::SpuFactoryError::ImageLoad`] when the image does not
/// load.
pub fn spu_unit(
    id: cellgov_event::UnitId,
    init: cellgov_lv2::SpuInitState,
) -> Result<Box<dyn cellgov_core::RegisteredUnit>, cellgov_core::SpuFactoryError> {
    use cellgov_spu::{loader as spu_loader, SpuExecutionUnit};
    let mut unit = SpuExecutionUnit::new(id);
    match &init.image {
        cellgov_lv2::SpuLoadImage::Elf(bytes) => {
            spu_loader::load_spu_elf(bytes, unit.state_mut()).map_err(|source| {
                cellgov_core::SpuFactoryError::ImageLoad {
                    detail: source.to_string(),
                }
            })?;
        }
        cellgov_lv2::SpuLoadImage::Segments(segments) => {
            let placed: Vec<(u32, &[u8])> = segments
                .iter()
                .map(|s| (s.ls_start, s.bytes.as_slice()))
                .collect();
            spu_loader::load_ls_segments(&placed, init.entry_pc, unit.state_mut()).map_err(
                |source| cellgov_core::SpuFactoryError::ImageLoad {
                    detail: source.to_string(),
                },
            )?;
        }
    }
    unit.state_mut().pc = init.entry_pc;
    unit.state_mut().set_reg_word_splat(1, init.stack_ptr);
    for (register, arg) in (3u8..).zip(init.args) {
        let mut quad = [0u8; 16];
        quad[..8].copy_from_slice(&arg.to_be_bytes());
        unit.state_mut().set_reg(usize::from(register), quad);
    }
    Ok(Box::new(unit))
}

/// Install the loader behind `_sys_process_spawn` /
/// `sys_process_spawns_a_self2`.
///
/// The runtime is mid-step when the loader runs, so a child's
/// `module_start`s cannot execute there: they are staged as a plan the
/// step loop runs before the child's primary thread is released. SELF
/// paths resolve through the LV2 content store.
///
/// Cross-module contract: the loader returns every refusal it can reach
/// as a `ProcessSpawnLoadError`, because it runs inside `Runtime::step`.
/// It can reach three:
///
/// - a key vault that will not load,
/// - a firmware set that will not close,
/// - a child image and firmware set that leave no TLS area in the
///   child's region.
///
/// The runtime rolls the spawn back, fails the syscall and logs the
/// cause (`cellgov_core` `process_spawn.rs`
/// `runtime.process_spawn_image_load_failed`); it never ends the run.
///
/// `profile_mode` turns on retirement counting in the child's
/// `module_start` units.
///
/// Each spawn withdraws `thread_shadow` before the child's image loads,
/// for the reason that [`ThreadShadow`] gives. A spawn that fails keeps
/// the shadow withdrawn.
pub(super) fn install_spawn_loader(
    rt: &mut Runtime,
    title: &TitleOptions<'_>,
    services: &BootServices,
    profile_mode: bool,
    thread_shadow: ThreadShadow,
) -> ChildInitPlans {
    let child_init = ChildInitPlans::default();
    let overrides = title.identity.overrides;
    let loader = SpawnLoader {
        thread_shadow,
        firmware: SpawnFirmware {
            dir: title.firmware_dir.map(str::to_string),
            // Scanned and decrypted on the first spawn; a boot that never
            // spawns pays nothing.
            candidates: RefCell::new(None),
            prx_base: overrides.prx_base,
        },
        plans: child_init.clone(),
        run_hle_stubbed: overrides.disable_module_start_hle_stubs,
        profile_mode,
        sink: Rc::clone(&services.sink),
        keys: Rc::clone(&services.keys),
        taps: Rc::clone(&services.taps),
    };
    rt.set_process_spawn_loader(move |elf_bytes, mem, space| loader.load(elf_bytes, mem, space));
    child_init
}

/// What the loader of [`install_spawn_loader`] keeps from one spawn to
/// the next.
struct SpawnLoader {
    thread_shadow: ThreadShadow,
    firmware: SpawnFirmware,
    /// The `module_start` plans that the step loop runs before it
    /// releases each child's primary thread.
    plans: ChildInitPlans,
    run_hle_stubbed: bool,
    profile_mode: bool,
    sink: Rc<dyn crate::BootSink>,
    keys: Rc<dyn crate::KeyVaultSource>,
    taps: Rc<dyn crate::DebugTaps>,
}

impl SpawnLoader {
    /// Loads the child image `elf_bytes` into `mem`, the memory of the
    /// child's new address space `space`.
    ///
    /// # Errors
    ///
    /// A [`cellgov_core::ProcessSpawnLoadError`] when one of these
    /// refuses the child:
    ///
    /// - the key vault;
    /// - the parse or the load of the child image;
    /// - the child's region or its code floor;
    /// - the firmware set;
    /// - the TLS area.
    fn load(
        &self,
        elf_bytes: &[u8],
        mem: &mut cellgov_mem::GuestMemory,
        space: cellgov_core::AddressSpaceId,
    ) -> Result<cellgov_core::SpawnedProcessImage, cellgov_core::ProcessSpawnLoadError> {
        self.thread_shadow.withdraw();
        let plaintext = decrypt_child_self(self.keys.as_ref(), elf_bytes)?;
        let elf_bytes: &[u8] = &plaintext;
        let required = cellgov_ppu::loader::required_memory_size(elf_bytes)
            .map_err(|e| image_parse(e.to_string()))?;
        let (child_mem_size, exit_stub_addr) = install_child_region_and_exit_stub(mem, required)?;
        let mut state = cellgov_ppu::state::PpuState::new();
        cellgov_ppu::loader::load_ppu_elf(elf_bytes, mem, &mut state)
            .map_err(|e| image_load(e.to_string()))?;
        let imports = cellgov_ppu::prx::parse_imports(elf_bytes)
            .map_err(|e| image_parse(format!("child imports: {e:?}")))?;
        let code_floor = u32::try_from(spawned_child_code_floor(required)).map_err(|_| {
            cellgov_core::ProcessSpawnLoadError::RegionSize {
                detail: format!("required_size=0x{required:x} leaves no 32-bit code floor"),
            }
        })?;
        let (prx_modules, child_exports) = load_child_firmware(
            &self.firmware,
            self.keys.as_ref(),
            &imports,
            mem,
            code_floor,
            self.sink.as_ref(),
        )?;
        let resident = ChildResident {
            image_end: required as u64,
            exit_stub_addr,
            region_size: child_mem_size as u64,
        };
        let tls = finish_child_space(elf_bytes, mem, &prx_modules, resident, self.sink.as_ref())?;
        // This report follows the last refusal above. On a refusal the
        // runtime removes the child's address space (`cellgov_core`
        // `process_spawn.rs` `handle_process_spawn`). An earlier report
        // can name OPDs in a space that the runtime then removes.
        if let Some(exports) = &child_exports {
            self.taps.firmware_bound(space.raw(), exports, mem);
        }

        let stack_top = (child_mem_size as u64) - 0x1000;
        let init_token = self.plans.stage(ChildInitPlan {
            prx_modules,
            tls,
            // A full primary-stack reservation below the child's SP:
            // the child's own stack grows down from `stack_top`, the
            // transient module_start stacks from here.
            stack_pointer: stack_top - u64::from(PS3_PRIMARY_STACK_SIZE_MAX),
            run_hle_stubbed: self.run_hle_stubbed,
            ppu_tap: self.taps.ppu(),
            profile_mode: self.profile_mode,
        });
        Ok(cellgov_core::SpawnedProcessImage {
            entry_code: state.pc,
            entry_toc: state.gpr[2],
            stack_top,
            lr_sentinel: exit_stub_addr,
            init_token: Some(init_token),
        })
    }
}

/// Where a spawned child's firmware set comes from: the firmware
/// directory, its candidates once scanned, and the PRX base override.
struct SpawnFirmware {
    dir: Option<String>,
    candidates: RefCell<Option<FirmwareCandidates>>,
    prx_base: Option<u64>,
}

/// The ends a spawned child's TLS area must sit above, and the region it
/// must fit inside.
#[derive(Clone, Copy)]
struct ChildResident {
    image_end: u64,
    exit_stub_addr: u64,
    region_size: u64,
}

/// Return a spawn refusal for an image that would not load.
fn image_load(detail: String) -> cellgov_core::ProcessSpawnLoadError {
    cellgov_core::ProcessSpawnLoadError::ImageLoad { detail }
}

/// Return a spawn refusal for an image that would not parse.
fn image_parse(detail: String) -> cellgov_core::ProcessSpawnLoadError {
    cellgov_core::ProcessSpawnLoadError::ImageParse { detail }
}

/// Unwrap a child image to its plaintext ELF.
///
/// A child image may arrive SCE-wrapped (vsh spawns SELFs, not raw
/// ELFs). The spawn loader is APP-keyed: klicensee resolution belongs to
/// the title-install layer, which is not reachable from inside the
/// runtime.
fn decrypt_child_self<'a>(
    keys: &dyn crate::KeyVaultSource,
    bytes: &'a [u8],
) -> Result<Cow<'a, [u8]>, cellgov_core::ProcessSpawnLoadError> {
    let vault = keys
        .vault_for(bytes)
        .map_err(|e| image_parse(format!("child SELF: key vault: {e}")))?;
    cellgov_install::self_image::to_plaintext_elf(
        bytes,
        vault,
        cellgov_install::self_image::KeyPolicy::AppOnly,
    )
    .map_err(|e| image_parse(format!("child SELF: {e}")))
}

/// Install a child's region in its space and write the exit stub the
/// child enters when its entry returns. Returns the region size and the
/// stub address.
fn install_child_region_and_exit_stub(
    mem: &mut cellgov_mem::GuestMemory,
    required: usize,
) -> Result<(usize, u64), cellgov_core::ProcessSpawnLoadError> {
    let child_mem_size = spawned_child_region_size(required)?;
    mem.install_region(0, child_mem_size, "spawned", cellgov_mem::PageSize::Page64K)
        .map_err(|source| cellgov_core::ProcessSpawnLoadError::RegionInstall { source })?;
    let exit_stub_addr = child_exit_stub_addr(required);
    write_exit_stub(mem, exit_stub_addr)?;
    Ok((child_mem_size, exit_stub_addr))
}

/// Write the exit stub at `exit_stub_addr` and declare it as code.
fn write_exit_stub(
    mem: &mut cellgov_mem::GuestMemory,
    exit_stub_addr: u64,
) -> Result<(), cellgov_core::ProcessSpawnLoadError> {
    // li r11, 22; sc -- the child enters this when its entry
    // returns. r11 is the LV2 syscall number, and 22 is
    // `cellgov_ps3_abi::lv2::syscall::PROCESS_EXIT`. The exit status is
    // whatever the entry left in r3.
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
    mem.declare_code(exit_stub_addr, stub.len() as u64);
    Ok(())
}

/// Return the firmware candidates in `cache`, scanning `dir` into it on
/// the first call.
fn get_or_scan<'c>(
    cache: &'c mut Option<FirmwareCandidates>,
    dir: &str,
    keys: &dyn crate::KeyVaultSource,
) -> Result<&'c FirmwareCandidates, cellgov_core::ProcessSpawnLoadError> {
    let candidates = match cache.take() {
        Some(candidates) => candidates,
        None => FirmwareCandidates::scan(dir, false, keys)
            .map_err(|e| image_load(format!("child firmware set: {e}")))?,
    };
    Ok(cache.insert(candidates))
}

/// Load a child's firmware set into its space, or, with no firmware
/// directory or an empty set, its unresolved-import trampolines.
/// Returns the loaded modules and the set's export map.
fn load_child_firmware(
    firmware: &SpawnFirmware,
    keys: &dyn crate::KeyVaultSource,
    imports: &[cellgov_ppu::prx::ImportedModule],
    mem: &mut cellgov_mem::GuestMemory,
    code_floor: u32,
    sink: &dyn crate::BootSink,
) -> Result<(Vec<PrxLoadInfo>, Option<ExportMap>), cellgov_core::ProcessSpawnLoadError> {
    let (mut prx_modules, child_exports) = match firmware.dir.as_deref() {
        Some(dir) => {
            let mut cache = firmware.candidates.borrow_mut();
            let candidates = get_or_scan(&mut cache, dir, keys)?;
            let (modules, _identity, host_link) = load_firmware_set_from(
                candidates,
                imports,
                mem,
                code_floor,
                firmware.prx_base,
                sink,
            )
            .map_err(|e| image_load(format!("child firmware set: {e}")))?;
            (modules, Some(host_link.exports))
        }
        None => (Vec::new(), None),
    };
    if prx_modules.is_empty() {
        let (info, _requesters) =
            install_unresolved_trampolines_only(imports, mem, u64::from(code_floor), sink)
                .map_err(|e| image_load(format!("child trampolines: {e}")))?;
        if let Some(info) = info {
            prx_modules.push(info);
        }
    }
    Ok((prx_modules, child_exports))
}

/// Place a child's TLS area, then fence the child's space.
///
/// The fence is [`super::image::fence_title_memory`], which the boot
/// also applies to the title's space. The kernel-context OPD in the TLS
/// area is the last code the loader writes, so the fence follows it.
fn finish_child_space(
    elf_bytes: &[u8],
    mem: &mut cellgov_mem::GuestMemory,
    prx_modules: &[PrxLoadInfo],
    resident: ChildResident,
    sink: &dyn crate::BootSink,
) -> Result<TlsArea, cellgov_core::ProcessSpawnLoadError> {
    let tls = place_child_tls(elf_bytes, mem, prx_modules, resident, sink)?;
    super::image::fence_title_memory(mem);
    Ok(tls)
}

/// Place a child's TLS area, initialize it, and install the
/// kernel-context OPD at its end.
///
/// The area takes the first 64 KB page at or above the end of
/// everything resident, as the boot's does. The exit stub counts as
/// resident: `sys_initialize_tls` writes the per-thread TLS header over
/// the first 0x30 bytes of the area.
fn place_child_tls(
    elf_bytes: &[u8],
    mem: &mut cellgov_mem::GuestMemory,
    prx_modules: &[PrxLoadInfo],
    resident: ChildResident,
    sink: &dyn crate::BootSink,
) -> Result<TlsArea, cellgov_core::ProcessSpawnLoadError> {
    let resident_end = prx_modules
        .iter()
        .map(|p| p.data_end)
        .max()
        .unwrap_or(0)
        .max(resident.image_end)
        .max(resident.exit_stub_addr + 8);
    let child_mem_size = resident.region_size;
    let tls = TlsArea::above(resident_end)
        .filter(|area| area.end() <= child_mem_size)
        .ok_or_else(|| cellgov_core::ProcessSpawnLoadError::RegionSize {
            detail: format!(
                "child image and firmware set end at 0x{resident_end:x}, leaving no TLS \
                 area inside the 0x{child_mem_size:x}-byte region",
            ),
        })?;
    pre_init_tls(elf_bytes, mem, tls, sink).map_err(|e| image_load(format!("child TLS: {e}")))?;
    install_kernel_context_opd(mem, tls)
        .map_err(|e| image_load(format!("child kernel-context OPD: {e}")))?;
    Ok(tls)
}

/// Address of a spawned child's exit stub: the landing site a child
/// enters when its entry returns instead of calling
/// `sys_process_exit`.
///
/// Derived from `required` (the highest PT_LOAD end), so no segment
/// of the image can land on it: `load_ppu_elf` writes only
/// `[p_vaddr, p_vaddr + p_memsz)` per PT_LOAD, never the segment's
/// `p_align` padding.
///
/// The floor of `STUB_MIN_ADDR` keeps address 0 out of the answer for
/// an image whose PT_LOADs total zero bytes. Address 0 is null
/// throughout the boot path, so a stub there would read as an unset
/// return site.
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

/// First page past a spawned child's image *and* its exit stub: the
/// floor its firmware set (`resolve_prx_base`) and unresolved-import
/// trampolines (`patch_got_atomic` at exactly this address) are placed
/// from.
///
/// A page-aligned image end puts the stub at `required` itself, so a
/// floor rounded from `required` would let the first trampoline OPD
/// overwrite the `li r11, 22; sc` the child returns into.
fn spawned_child_code_floor(required: usize) -> u64 {
    // `required_memory_size` caps every segment end at the 4 GiB EA
    // ceiling, so the stub end and this round-up stay far inside u64.
    (child_exit_stub_addr(required) + 8).next_multiple_of(0x1000)
}

/// Sizes a spawned child's address-space region from its ELF's
/// required memory by the rule the boot's own main region uses.
///
/// The floor leaves room past the image for the child's firmware set,
/// its TLS area and its stack.
/// The child's initial SP is one page below the region end.
fn spawned_child_region_size(
    required: usize,
) -> Result<usize, cellgov_core::ProcessSpawnLoadError> {
    const PRX_HEADROOM: usize = 0x20_0000;
    const CHILD_MEM_FLOOR: usize = 0x4000_0000;
    let size = required
        .checked_add(0xFFFF)
        .map(|v| v & !0xFFFF)
        .and_then(|v| v.checked_add(PRX_HEADROOM))
        .ok_or_else(|| cellgov_core::ProcessSpawnLoadError::RegionSize {
            detail: format!("required_size=0x{required:x} overflows usize"),
        })?
        .max(CHILD_MEM_FLOOR);
    if size as u64 > PS3_RSX_IOMAP_BASE {
        return Err(cellgov_core::ProcessSpawnLoadError::RegionSize {
            detail: format!(
                "required_size=0x{required:x} needs a 0x{size:x} region, past \
                 PS3_RSX_IOMAP_BASE 0x{PS3_RSX_IOMAP_BASE:x}"
            ),
        });
    }
    Ok(size)
}

#[cfg(test)]
#[path = "tests/loaders_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/child_fence_tests.rs"]
mod child_fence_tests;

#[cfg(test)]
#[path = "tests/thread_shadow_tests.rs"]
mod thread_shadow_tests;

#[cfg(test)]
#[path = "tests/spawn_shadow_tests.rs"]
mod spawn_shadow_tests;
