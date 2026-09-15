//! The boot's stage sequence.

use std::time::Instant;

use super::types::{BootDebugOptions, PrepareOptions, PreparedBoot, StartupTimings};
use super::{entry, finish, firmware, host, image, loaders, module_start, params, providers};
use crate::prx::{install_kernel_context_opd, pre_init_tls};
use crate::BootError;

/// Bring a title from decrypted ELF bytes to a runtime whose primary
/// unit is one `step()` from its first instruction.
///
/// The order below is the boot's contract; each stage reads state the
/// ones above it produced. Three orderings are not free to move:
/// the runtime and LV2 host exist before any `module_start`, so every
/// PRX init runs in the host the title later runs against; the primary
/// unit registers before them, so their transient units have a
/// `PpuThreadId` to alias; and the debug patches land after them, so
/// they override the memory the title actually sees.
///
/// # Errors
///
/// Any stage's refusal, as the [`BootError`] variant naming it. Nothing
/// here ends the process; a refusal drops the partly-built runtime.
pub fn prepare(opts: PrepareOptions<'_>) -> Result<PreparedBoot, BootError> {
    let PrepareOptions {
        mut title,
        execution,
        diagnostics,
        services,
    } = opts;
    let t_start = Instant::now();
    let elf_data = std::mem::take(&mut title.elf_data);
    let sink = services.sink.as_ref();

    // 1. Guest address space, with the title image loaded into it.
    let mut image = image::load_image(&title, &execution, &diagnostics, sink, &elf_data, t_start)?;

    // 2. The firmware PRX set the title's import tables name.
    let firmware::FirmwareSet {
        prx_modules,
        verified,
        host_link,
        t_hle_bind,
        t_prx_load,
    } = firmware::load_firmware_set(
        &title,
        &diagnostics,
        &services,
        &elf_data,
        &mut image.mem,
        image.code_floor,
        t_start,
    )?;
    pre_init_tls(&elf_data, &mut image.mem, sink)?;

    // 3. The guest heap floor, above everything now resident.
    let placement = firmware::place_guest_heap(&elf_data, &prx_modules, image.code_floor, sink)?;
    // The kernel-context OPD lives in the TLS reservation and is
    // consumed by every PRX's module_start entry (r11/r12). Install
    // once before the module_start loop.
    let kctx_opd = install_kernel_context_opd(&mut image.mem)?;

    // 4. Process parameters: step budget, priority, stack, TLS.
    let params = params::resolve_boot_params(&execution, sink, &elf_data)?;

    // 5. What the boot loaded.
    if diagnostics.print_banner {
        host::report_boot_banner(&title, &execution, sink, &image, &params, &prx_modules);
    }

    // 6. Runtime and LV2 host.
    let (mut rt, authid_source) = host::build_runtime(
        image.mem,
        &title,
        sink,
        &params,
        placement.alloc_base,
        verified.as_ref(),
        host_link,
    );
    host::register_prx_modules(&mut rt, &prx_modules)?;

    // 7. Execution-unit factories, inherited by guest-created threads.
    let debug_opts = BootDebugOptions {
        dump_at_pc: diagnostics.dump_at_pc,
        dump_skip: diagnostics.dump_skip,
        profile_pairs: diagnostics.profile_pairs,
    };
    loaders::install_unit_factories(&mut rt, debug_opts);

    // 8. Spawned-child image loader.
    let child_init = loaders::install_spawn_loader(&mut rt, &title, &services);

    // 9. Guest-visible content, mounts last.
    providers::register_sibling_images(&mut rt, title.elf_path)?;
    providers::register_content(&mut rt, &title, &diagnostics, sink)?;
    providers::register_mounts(&mut rt, &title, &diagnostics, sink)?;

    // 10. The title's primary entry state.
    let mut state = image.state;
    entry::seed_primary_entry_state(
        &mut rt,
        &mut state,
        &execution,
        &diagnostics,
        sink,
        &params,
        image.entry,
    )?;

    // 11. Predecode shadow over the entry state's executable code.
    let shadow = entry::build_instruction_shadow(&rt, &placement, image.code_floor, sink)?;

    // 12. The primary unit, held non-runnable until module_start ends.
    let primary_unit_id =
        entry::register_primary_unit(&mut rt, state, shadow, debug_opts, &params, image.entry)?;

    // 13. Every loaded PRX's module_start.
    let counts = module_start::run_module_starts(
        &mut rt,
        &prx_modules,
        title.identity.overrides,
        &diagnostics,
        &services,
        primary_unit_id,
        kctx_opd,
    )?;

    // 14. Debug patches and dumps.
    finish::apply_patch_bytes(&mut rt, &execution, &diagnostics, sink)?;
    finish::dump_boot_memory(&rt, diagnostics.dump_mem_boot_addrs, sink);

    // 15. Invariants, then release the primary to the step loop.
    finish::assert_gating_state_coherent_with_host(&rt, !prx_modules.is_empty());
    finish::assert_module_start_completeness(&counts);
    rt.clear_unit_status_override(primary_unit_id);

    Ok(PreparedBoot {
        rt,
        elf_data,
        child_init,
        timings: StartupTimings {
            mem_alloc: image.t_mem_alloc,
            elf_load: image.t_elf_load - image.t_mem_alloc,
            hle_bind: t_hle_bind - image.t_elf_load,
            prx_load: t_prx_load - t_hle_bind,
        },
        step_budget: params.step_budget,
        authid_source,
    })
}
