//! The boot banner, and the runtime plus LV2 host that every firmware
//! `module_start` and the title itself run against.

use cellgov_core::Runtime;

use super::image::LoadedImage;
use super::params::BootParams;
use super::types::{u32_or_die, AuthorityIdSource, PrepareOptions};
use crate::cli::env::parse_env_bool;
use crate::cli::exit::die;
use crate::game::prx::{HostLinkMaps, PrxLoadInfo, VerifiedFirmware};

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

/// What the boot loaded, printed before the runtime exists.
pub(super) fn print_boot_banner(
    opts: &PrepareOptions<'_>,
    image: &LoadedImage,
    params: &BootParams,
    prx_modules: &[PrxLoadInfo],
) {
    println!("title: {}", opts.title.display_name());
    println!("elf: {}", opts.elf_path);
    println!("memory: {} MB", image.mem_size / (1024 * 1024));
    println!(
        "entry: 0x{:x} (OPD) -> pc=0x{:x} toc=0x{:x}",
        image.entry, image.state.pc, image.state.gpr[2]
    );
    if let Some(p) = params.proc_param {
        println!(
            "sys_proc_param: sdk=0x{:x} prio={} stack=0x{:x} malloc_pagesize=0x{:x}",
            p.sdk_version, p.primary_prio, p.primary_stacksize, p.malloc_pagesize,
        );
    } else {
        println!(
            "sys_proc_param: not found, using malloc_pagesize=0x{:x}",
            params.malloc_pagesize
        );
    }
    for info in prx_modules {
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
    println!("budget: {} ({budget_source})", params.step_budget);
    println!();
}

/// Build the runtime and bind the boot's identity into its LV2 host.
///
/// Runs BEFORE any `module_start` so every PRX's init runs in the same
/// host the title later runs against.
pub(super) fn build_runtime(
    mem: cellgov_mem::GuestMemory,
    opts: &PrepareOptions<'_>,
    params: &BootParams,
    alloc_base: u32,
    verified_firmware: Option<&VerifiedFirmware>,
    host_link: HostLinkMaps,
) -> (Runtime, AuthorityIdSource) {
    let mut rt = Runtime::new(mem, params.step_budget, params.adjusted_max_steps);
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
            AuthorityIdSource::Forced,
        )
    } else if opts.authority_id.is_some() {
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
        params
            .proc_param
            .map(|p| p.sdk_version)
            .unwrap_or(cellgov_ps3_abi::elf::SYS_PROCESS_PARAM_SDK_VERSION_UNKNOWN),
        if params.proc_param.is_some() {
            "from sys_proc_param segment"
        } else {
            "absent -- PSL1GHT homebrew sentinel"
        },
    );
    (rt, authid_source)
}

/// Publish each loaded firmware module in the host's PRX registry,
/// already marked started.
///
/// Cross-module contract: firmware-side `_sys_prx_load_module(path)`
/// resolves the guest path against this registry to recover the
/// kernel id; an empty stem makes the module unreachable from
/// libsysmodule's load worker.
pub(super) fn register_prx_modules(rt: &mut Runtime, prx_modules: &[PrxLoadInfo]) {
    for info in prx_modules {
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
        // Boot runs every firmware module's module_start, so these
        // enter the resident/started state LV2 refuses to unload;
        // only sc 480 miss stubs stay unstarted.
        rt.lv2_host_mut().prx_registry_mut().mark_started(id);
    }
}

#[cfg(test)]
#[path = "tests/host_tests.rs"]
mod tests;
