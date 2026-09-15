//! The closures the runtime calls back into -- the PPU and SPU
//! execution-unit factories, and the spawned-child image loader.

use std::cell::RefCell;
use std::rc::Rc;

use cellgov_core::Runtime;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_ps3_abi::hw::address_space::{PS3_PRIMARY_STACK_SIZE, PS3_RSX_IOMAP_BASE};

use super::types::{BootDebugOptions, PrepareOptions};
use crate::child_init::{ChildInitPlan, ChildInitPlans};
use crate::prx::{
    install_kernel_context_opd, install_unresolved_trampolines_only, load_firmware_set_from,
    pre_init_tls, FirmwareCandidates, TLS_BASE,
};

/// Install the factories the runtime materializes execution units from.
///
/// Both fire again for every child the guest creates, so `debug_opts`
/// is captured by the closure.
pub(super) fn install_unit_factories(rt: &mut Runtime, debug_opts: BootDebugOptions) {
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
        debug_opts.apply(&mut unit);
        Box::new(unit)
    });
    // Cell BE convention: args 0..3 map to r3..r6 (arg0 -> r3, etc.).
    rt.set_spu_factory(|id, init| {
        use cellgov_spu::{loader as spu_loader, SpuExecutionUnit};
        let mut unit = SpuExecutionUnit::new(id);
        match &init.image {
            cellgov_lv2::SpuLoadImage::Elf(bytes) => {
                spu_loader::load_spu_elf(bytes, unit.state_mut())
                    .expect("game boot: load_spu_elf on title-provided ELF; failure indicates a bad LV2 thread init");
            }
            cellgov_lv2::SpuLoadImage::Segments(segments) => {
                let placed: Vec<(u32, &[u8])> = segments
                    .iter()
                    .map(|s| (s.ls_start, s.bytes.as_slice()))
                    .collect();
                spu_loader::load_ls_segments(&placed, init.entry_pc, unit.state_mut())
                    .expect("game boot: load_ls_segments on segments sys_spu_thread_initialize already bounded");
            }
        }
        unit.state_mut().pc = init.entry_pc;
        unit.state_mut().set_reg_word_splat(1, init.stack_ptr);
        unit.state_mut().set_reg_word_splat(3, init.args[0] as u32);
        unit.state_mut().set_reg_word_splat(4, init.args[1] as u32);
        unit.state_mut().set_reg_word_splat(5, init.args[2] as u32);
        unit.state_mut().set_reg_word_splat(6, init.args[3] as u32);
        Box::new(unit)
    });
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
/// - a child image past the TLS reservation.
///
/// The runtime rolls the spawn back, fails the syscall and logs the
/// cause (`cellgov_core` `process_spawn.rs`
/// `runtime.process_spawn_image_load_failed`); it never ends the run.
pub(super) fn install_spawn_loader(rt: &mut Runtime, opts: &PrepareOptions<'_>) -> ChildInitPlans {
    let child_init = ChildInitPlans::default();
    let spawn_firmware_dir: Option<String> = opts.firmware_dir.map(str::to_string);
    // Scanned and decrypted on the first spawn; a boot that never
    // spawns pays nothing.
    let spawn_candidates: RefCell<Option<FirmwareCandidates>> = RefCell::new(None);
    let loader_plans = child_init.clone();
    let sink: Rc<dyn crate::BootSink> = Rc::clone(&opts.sink);
    let keys: Rc<dyn crate::KeyVaultSource> = Rc::clone(&opts.keys);
    rt.set_process_spawn_loader(move |elf_bytes, mem| {
        // A child image may arrive SCE-wrapped (vsh spawns SELFs, not
        // raw ELFs). The spawn loader is APP-keyed: klicensee
        // resolution belongs to the title-install layer, which is not
        // reachable from inside the runtime.
        let vault = keys.vault_for(elf_bytes).map_err(|e| {
            cellgov_core::ProcessSpawnLoadError::ImageParse {
                detail: format!("child SELF: key vault: {e}"),
            }
        })?;
        let plaintext = cellgov_install::self_image::to_plaintext_elf(
            elf_bytes,
            vault,
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
        let mut state = cellgov_ppu::state::PpuState::new();
        cellgov_ppu::loader::load_ppu_elf(elf_bytes, mem, &mut state).map_err(|e| {
            cellgov_core::ProcessSpawnLoadError::ImageLoad {
                detail: e.to_string(),
            }
        })?;

        let imports = cellgov_ppu::prx::parse_imports(elf_bytes).map_err(|e| {
            cellgov_core::ProcessSpawnLoadError::ImageParse {
                detail: format!("child imports: {e:?}"),
            }
        })?;
        let code_floor = u32::try_from(spawned_child_code_floor(required)).map_err(|_| {
            cellgov_core::ProcessSpawnLoadError::RegionSize {
                detail: format!("required_size=0x{required:x} leaves no 32-bit code floor"),
            }
        })?;
        let mut prx_modules = match spawn_firmware_dir.as_deref() {
            Some(dir) => {
                let mut cache = spawn_candidates.borrow_mut();
                let candidates =
                    match cache.as_ref() {
                        Some(c) => c,
                        None => {
                            let scanned = FirmwareCandidates::scan(dir, false, keys.as_ref())
                                .map_err(|e| cellgov_core::ProcessSpawnLoadError::ImageLoad {
                                    detail: format!("child firmware set: {e}"),
                                })?;
                            cache.insert(scanned)
                        }
                    };
                let (modules, _identity, _host_link) =
                    load_firmware_set_from(candidates, &imports, mem, code_floor, sink.as_ref())
                        .map_err(|e| cellgov_core::ProcessSpawnLoadError::ImageLoad {
                            detail: format!("child firmware set: {e}"),
                        })?;
                modules
            }
            None => Vec::new(),
        };
        if prx_modules.is_empty() {
            let (info, _requesters) = install_unresolved_trampolines_only(
                &imports,
                mem,
                u64::from(code_floor),
                sink.as_ref(),
            )
            .map_err(|e| cellgov_core::ProcessSpawnLoadError::ImageLoad {
                detail: format!("child trampolines: {e}"),
            })?;
            if let Some(info) = info {
                prx_modules.push(info);
            }
        }
        // TLS, the kernel-context OPD and the HLE heap sit at fixed
        // addresses above `TLS_BASE` in every process; an image or
        // firmware set reaching them cannot be initialised there.
        // The exit stub counts as image: the 0x30 bytes at `TLS_BASE`
        // are the per-thread TLS header `sys_initialize_tls` writes.
        let image_end = prx_modules
            .iter()
            .map(|p| p.data_end)
            .max()
            .unwrap_or(0)
            .max(required as u64)
            .max(exit_stub_addr + 8);
        if image_end > TLS_BASE {
            return Err(cellgov_core::ProcessSpawnLoadError::RegionSize {
                detail: format!(
                    "child image and firmware set end at 0x{image_end:x}, past the TLS \
                     reservation at 0x{TLS_BASE:x}",
                ),
            });
        }
        pre_init_tls(elf_bytes, mem, sink.as_ref()).map_err(|e| {
            cellgov_core::ProcessSpawnLoadError::ImageLoad {
                detail: format!("child TLS: {e}"),
            }
        })?;
        let kctx_opd = install_kernel_context_opd(mem).map_err(|e| {
            cellgov_core::ProcessSpawnLoadError::ImageLoad {
                detail: format!("child kernel-context OPD: {e}"),
            }
        })?;

        let stack_top = (child_mem_size as u64) - 0x1000;
        let init_token = loader_plans.stage(ChildInitPlan {
            prx_modules,
            kctx_opd,
            // A full primary-stack reservation below the child's SP:
            // the child's own stack grows down from `stack_top`, the
            // transient module_start stacks from here.
            stack_pointer: stack_top - PS3_PRIMARY_STACK_SIZE as u64,
        });
        Ok(cellgov_core::SpawnedProcessImage {
            entry_code: state.pc,
            entry_toc: state.gpr[2],
            stack_top,
            lr_sentinel: exit_stub_addr,
            init_token: Some(init_token),
        })
    });
    child_init
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
/// The floor is what lets the child share the boot's fixed layout:
/// TLS at `TLS_BASE`, the kernel-context OPD and HLE heap above it,
/// and a firmware set placed past the image all sit far below 1 GiB.
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
