//! The boot banner, and the runtime plus LV2 host that every firmware
//! `module_start` and the title itself run against.

use cellgov_core::Runtime;

use super::image::LoadedImage;
use super::params::BootParams;
use super::types::{AuthorityIdSource, ExecutionOptions, TitleOptions};
use crate::error::narrow_u32;
use crate::prx::{HostLinkMaps, PrxLoadInfo, VerifiedFirmware};
use crate::BootError;

/// Why the runtime or its LV2 host could not be bound.
#[derive(Debug, thiserror::Error)]
pub enum HostBindError {
    /// A loaded module carries no filesystem stem, so the PRX registry
    /// could not reach it by path.
    #[error(
        "prx: module {name:?} loaded with empty stem; registry would not reach it via path lookup"
    )]
    EmptyStem {
        /// The module's own name from its PRX header.
        name: String,
    },
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
    use cellgov_ps3_abi::lv2::ipc::{
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

/// What the boot loaded, reported before the runtime exists.
pub(super) fn report_boot_banner(
    title: &TitleOptions<'_>,
    execution: &ExecutionOptions<'_>,
    sink: &dyn crate::BootSink,
    image: &LoadedImage,
    params: &BootParams,
    prx_modules: &[PrxLoadInfo],
) {
    sink.note(&format!("title: {}", title.manifest.display_name()));
    sink.note(&format!("elf: {}", title.elf_path));
    sink.note(&format!("memory: {} MB", image.mem_size / (1024 * 1024)));
    sink.note(&format!(
        "entry: 0x{:x} (OPD) -> pc=0x{:x} toc=0x{:x}",
        image.entry, image.state.pc, image.state.gpr[2]
    ));
    if let Some(p) = params.proc_param {
        sink.note(&format!(
            "sys_proc_param: sdk=0x{:x} prio={} stack=0x{:x} malloc_pagesize=0x{:x}",
            p.sdk_version, p.primary_prio, p.primary_stacksize, p.malloc_pagesize,
        ));
    } else {
        sink.note(&format!(
            "sys_proc_param: not found, using malloc_pagesize=0x{:x}",
            params.malloc_pagesize
        ));
    }
    for info in prx_modules {
        sink.note(&format!(
            "prx: {} at 0x{:x} (toc=0x{:x}, {} relocs)",
            info.name, info.base, info.toc, info.relocs_applied,
        ));
    }
    sink.note(&format!("max_steps: {}", execution.runtime_max_steps));
    let budget_source = if execution.budget_override.is_some() {
        "override"
    } else {
        "mode-default"
    };
    sink.note(&format!("budget: {} ({budget_source})", params.step_budget));
    sink.note("");
}

/// Build the runtime and bind the boot's identity into its LV2 host.
///
/// Runs before any `module_start` so every PRX's init runs in the same
/// host the title later runs against.
pub(super) fn build_runtime(
    mem: cellgov_mem::GuestMemory,
    title: &TitleOptions<'_>,
    sink: &dyn crate::BootSink,
    params: &BootParams,
    alloc_base: u32,
    verified_firmware: Option<&VerifiedFirmware>,
    host_link: HostLinkMaps,
) -> (Runtime, AuthorityIdSource) {
    // The header leads the stream, so the writer takes it before the
    // runtime that appends to it exists.
    let mut trace = cellgov_trace::TraceWriter::new();
    trace.record_header(&title.identity.trace_header());
    let mut rt =
        Runtime::with_trace_writer(mem, params.step_budget, params.adjusted_max_steps, trace);
    rt.set_mode(params.mode);
    rt.lv2_host_mut().set_mem_alloc_base(alloc_base);
    // Bind the manifest-verified PUP identity so the boot's state hash
    // is a function of which firmware revision fed it.
    if let Some(fw) = verified_firmware {
        rt.lv2_host_mut()
            .set_firmware_identity(&fw.image_version, fw.pup_sha256);
    }
    // Plumb the title's recorded SDK version into the LV2 host so
    // `sys_process_get_sdk_version` reports the value cellSysutil's
    // SDK-keyed init dispatcher gates on. An absent param segment
    // leaves the PSL1GHT homebrew sentinel in place.
    if let Some(p) = params.proc_param {
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
    if let Some(authid) = title.authority_id {
        rt.lv2_host_mut().set_program_authority_id(authid);
    }
    // Adversarial knob for the authority-id tripwire test: forcing
    // the bdj.self system authid makes the cellSysmodule
    // LoadModule-failure signature reappear.
    let (authid_label, authid_source) = if title.identity.overrides.force_system_authid {
        rt.lv2_host_mut()
            .set_program_authority_id(cellgov_ps3_abi::format::sce::BDJ_SELF_PROGRAM_AUTHORITY_ID);
        (
            "forced system authid (boot override force_system_authid)",
            AuthorityIdSource::Forced,
        )
    } else if title.authority_id.is_some() {
        (
            "from SELF identification header",
            AuthorityIdSource::SelfHeader,
        )
    } else {
        (
            "raw-ELF input -- retail-application fallback",
            AuthorityIdSource::Fallback,
        )
    };
    sink.note(&format!(
        "program_authority_id: 0x{:016x} ({})",
        rt.lv2_host().program_authority_id(),
        authid_label,
    ));
    // Process privilege, from the SELF's plaintext capability header.
    if let Some(flags) = title.control_flags1 {
        rt.lv2_host_mut().set_control_flags1(flags);
    }
    // Resolution source for the sc 484 CoreOS manual import link, and
    // the requester map behind the unresolved-import diagnostic.
    rt.lv2_host_mut().set_firmware_exports(host_link.exports);
    rt.lv2_host_mut()
        .set_unresolved_import_requesters(host_link.unresolved_requesters);
    {
        let h = rt.lv2_host();
        sink.note(&format!(
            "ctrl_flags1: 0x{:08x} (root={} debug_or_root={} debug={} coreos={})",
            h.control_flags1(),
            h.has_root_perm(),
            h.debug_or_root(),
            h.has_debug_perm(),
            h.is_coreos(),
        ));
    }
    sink.note(&format!(
        "process_param: sdk_version=0x{:08x} ({})",
        params
            .proc_param
            .map(|p| p.sdk_version)
            .unwrap_or(cellgov_ps3_abi::format::elf::SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN),
        if params.proc_param.is_some() {
            "from sys_proc_param segment"
        } else {
            "absent -- PSL1GHT homebrew sentinel"
        },
    ));
    (rt, authid_source)
}

/// Publish each loaded firmware module in the host's PRX registry,
/// already marked started.
///
/// Cross-module contract: firmware-side `_sys_prx_load_module(path)`
/// resolves the guest path against this registry to recover the
/// kernel id; an empty stem makes the module unreachable from
/// libsysmodule's load worker.
pub(super) fn register_prx_modules(
    rt: &mut Runtime,
    prx_modules: &[PrxLoadInfo],
) -> Result<(), BootError> {
    for info in prx_modules {
        if info.is_synthetic() {
            continue;
        }
        if info.stem.is_empty() {
            return Err(HostBindError::EmptyStem {
                name: info.name.clone(),
            }
            .into());
        }
        // Narrowed in field order: a module with two fields too wide
        // for the registry is refused by the first of them.
        let base = narrow_u32("prx base", info.base)?;
        let data_end = narrow_u32("prx data_end", info.data_end)?;
        let toc = narrow_u32("prx toc", info.toc)?;
        let module_start = info
            .module_start
            .map(|opd| narrow_u32("prx module_start", opd.code))
            .transpose()?;
        let module_stop = info
            .module_stop
            .map(|opd| narrow_u32("prx module_stop", opd.code))
            .transpose()?;
        let id = rt.lv2_host_mut().prx_registry_mut().register(
            info.stem.clone(),
            info.name.clone(),
            base,
            data_end,
            toc,
            module_start,
            module_stop,
        );
        // Boot runs every firmware module's module_start, so these
        // enter the resident/started state LV2 refuses to unload;
        // only sc 480 miss stubs stay unstarted.
        rt.lv2_host_mut().prx_registry_mut().mark_started(id);
    }
    Ok(())
}

#[cfg(test)]
#[path = "tests/host_tests.rs"]
mod tests;
