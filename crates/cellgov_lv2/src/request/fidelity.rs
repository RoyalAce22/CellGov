//! Per-arm fidelity of the LV2 dispatch surface.
//!
//! Two claims with different evidence standards, kept separate:
//!
//! - **Routing layer**: no syscall reaches the guest with a result
//!   produced by a default arm. Enforced by dispatch structure --
//!   unhandled numbers hit the null backend and return `CELL_ENOSYS`.
//! - **Per-arm fidelity**: how faithful each modeled arm is. A
//!   per-arm claim, individually auditable; that is this map.
//!
//! [`Lv2RequestKind::fidelity`] is an exhaustive match, so a new
//! request variant fails to compile until it declares a tag.
//! `docs/lv2_fidelity.md` renders this map; the `fidelity_doc`
//! integration test fails when the committed doc drifts.

use super::Lv2RequestKind;

/// How much of the real LV2 behavior a dispatch arm reproduces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArmFidelity {
    /// Full modeled state and ABI, faithful to the oracle.
    Modeled,
    /// ABI (return codes, out-writes) faithful; some kernel-visible
    /// state simplified or omitted, documented on the arm.
    PartialState,
    /// Plausible return value with little or no backing state; a
    /// read-back or cross-check against real LV2 would diverge.
    AbiOnly,
    /// Honest refusal: `CELL_ENOSYS`-class response with a logged
    /// diagnostic, no result fabricated.
    NullBackend,
}

impl ArmFidelity {
    /// Stable lowercase label used by the rendered doc.
    pub fn label(self) -> &'static str {
        match self {
            ArmFidelity::Modeled => "modeled",
            ArmFidelity::PartialState => "partial-state",
            ArmFidelity::AbiOnly => "abi-only",
            ArmFidelity::NullBackend => "null-backend",
        }
    }
}

/// Syscalls routed inside [`super::Lv2Request::Unsupported`] to a
/// dedicated arm, with their fidelity. Numbers absent from this table
/// dispatch to the null backend.
///
/// # Cross-module contract
///
/// Rows mirror the `Unsupported { number: syscall::X }` match arms in
/// `host/dispatch_route/dispatch.rs`. The
/// `routed_unsupported_fidelity_table_matches_dispatch_exactly` probe
/// dispatches every slot and fails when the two sets diverge.
pub const ROUTED_UNSUPPORTED_ARMS: &[(u64, &str, ArmFidelity)] = {
    use cellgov_ps3_abi::syscall;
    &[
        (
            syscall::PPU_THREAD_GET_PRIORITY,
            "sys_ppu_thread_get_priority",
            ArmFidelity::Modeled,
        ),
        (
            syscall::EVENT_PORT_CONNECT_LOCAL,
            "sys_event_port_connect_local",
            ArmFidelity::Modeled,
        ),
        (
            syscall::EVENT_PORT_DISCONNECT,
            "sys_event_port_disconnect",
            ArmFidelity::Modeled,
        ),
        (
            syscall::EVENT_PORT_CONNECT_IPC,
            "sys_event_port_connect_ipc",
            ArmFidelity::Modeled,
        ),
        (
            syscall::MEMORY_CONTAINER_CREATE_324,
            "sys_memory_container_create",
            ArmFidelity::AbiOnly,
        ),
        (
            syscall::MMAPPER_ALLOCATE_ADDRESS,
            "sys_mmapper_allocate_address",
            ArmFidelity::PartialState,
        ),
        (
            syscall::MMAPPER_ALLOCATE_SHARED_MEMORY,
            "sys_mmapper_allocate_shared_memory",
            ArmFidelity::PartialState,
        ),
        (
            syscall::MMAPPER_MAP_SHARED_MEMORY,
            "sys_mmapper_map_shared_memory",
            ArmFidelity::PartialState,
        ),
        (
            syscall::MMAPPER_SEARCH_AND_MAP,
            "sys_mmapper_search_and_map",
            ArmFidelity::PartialState,
        ),
        (
            syscall::MMAPPER_ALLOCATE_SHARED_MEMORY_FROM_CONTAINER,
            "sys_mmapper_allocate_shared_memory_from_container",
            ArmFidelity::PartialState,
        ),
        (syscall::TTY_READ, "sys_tty_read", ArmFidelity::Modeled),
        (
            syscall::UNS_FUNC_462,
            "uns_func slot 462 (DEX-only)",
            ArmFidelity::Modeled,
        ),
        (
            syscall::SYS_PRX_LOAD_MODULE,
            "_sys_prx_load_module",
            ArmFidelity::PartialState,
        ),
        (
            syscall::SYS_PRX_START_MODULE,
            "_sys_prx_start_module",
            ArmFidelity::PartialState,
        ),
        (
            syscall::SYS_PRX_STOP_MODULE,
            "_sys_prx_stop_module",
            ArmFidelity::PartialState,
        ),
        (
            syscall::SYS_PRX_UNLOAD_MODULE,
            "_sys_prx_unload_module",
            ArmFidelity::PartialState,
        ),
        (
            syscall::SYS_PRX_REGISTER_MODULE,
            "_sys_prx_register_module",
            ArmFidelity::Modeled,
        ),
        (
            syscall::SYS_PRX_REGISTER_LIBRARY,
            "_sys_prx_register_library",
            ArmFidelity::PartialState,
        ),
        (
            syscall::SYS_PRX_GET_MODULE_LIST,
            "_sys_prx_get_module_list",
            ArmFidelity::PartialState,
        ),
        (
            syscall::SYS_PRX_LOAD_MODULE_ON_MEMCONTAINER,
            "_sys_prx_load_module_on_memcontainer",
            ArmFidelity::PartialState,
        ),
        (
            syscall::HID_IS_ROOT,
            "sys_hid_manager_is_process_permission_root",
            ArmFidelity::Modeled,
        ),
        (
            syscall::GAMEPAD_YCON_IF,
            "sys_gamepad_ycon_if",
            ArmFidelity::AbiOnly,
        ),
        (
            syscall::RSX_ATTRIBUTE,
            "sys_rsx_attribute",
            ArmFidelity::AbiOnly,
        ),
    ]
};

/// Fidelity of the arm a routed `Unsupported { number }` reaches.
/// Numbers outside [`ROUTED_UNSUPPORTED_ARMS`] hit the null backend.
pub fn unsupported_arm_fidelity(number: u64) -> ArmFidelity {
    ROUTED_UNSUPPORTED_ARMS
        .iter()
        .find(|(n, _, _)| *n == number)
        .map(|(_, _, f)| *f)
        .unwrap_or(ArmFidelity::NullBackend)
}

impl Lv2RequestKind {
    /// Fidelity of this arm, or `None` for the routing-layer variants
    /// (`Unsupported` has per-number fidelity via
    /// [`unsupported_arm_fidelity`]; `Hypercall`, `Malformed`, and
    /// `UnresolvedImport` are rejections, not modeled syscalls).
    pub fn fidelity(self) -> Option<ArmFidelity> {
        use ArmFidelity::{AbiOnly, Modeled, NullBackend, PartialState};
        Some(match self {
            // Routing layer: not arms.
            Lv2RequestKind::Unsupported
            | Lv2RequestKind::Hypercall
            | Lv2RequestKind::UnresolvedImport
            | Lv2RequestKind::Malformed => return None,

            // SPU lifecycle.
            Lv2RequestKind::SpuImageOpen
            | Lv2RequestKind::SpuImageImport
            | Lv2RequestKind::SpuThreadGroupCreate
            | Lv2RequestKind::SpuThreadInitialize
            | Lv2RequestKind::SpuThreadGroupStart
            | Lv2RequestKind::SpuThreadGroupDestroy
            | Lv2RequestKind::SpuThreadGroupJoin
            | Lv2RequestKind::SpuThreadWriteMb => Modeled,
            // SPU teardown is an honest ENOSYS refusal.
            Lv2RequestKind::SpuThreadGroupTerminate => NullBackend,
            // Announced limits are validated but not persisted.
            Lv2RequestKind::SpuInitialize => PartialState,

            // Deterministic guest time is the model.
            Lv2RequestKind::TimeGetCurrentTime | Lv2RequestKind::TimeGetTimebaseFrequency => {
                Modeled
            }
            // Fixed UTC zeros; console timezone config not modeled.
            Lv2RequestKind::TimeGetTimezone => AbiOnly,

            // Retail LV2 discards TTY output and reports len written;
            // the tty_log capture is instrument-side.
            Lv2RequestKind::TtyWrite => Modeled,

            // Sync primitives on the shared WaiterList protocol.
            Lv2RequestKind::MutexCreate
            | Lv2RequestKind::MutexDestroy
            | Lv2RequestKind::MutexLock
            | Lv2RequestKind::MutexUnlock
            | Lv2RequestKind::MutexTryLock
            | Lv2RequestKind::SemaphoreCreate
            | Lv2RequestKind::SemaphoreDestroy
            | Lv2RequestKind::SemaphoreWait
            | Lv2RequestKind::SemaphorePost
            | Lv2RequestKind::SemaphoreTryWait
            | Lv2RequestKind::SemaphoreGetValue
            | Lv2RequestKind::EventQueueCreate
            | Lv2RequestKind::EventQueueDestroy
            | Lv2RequestKind::EventQueueReceive
            | Lv2RequestKind::EventQueueTryReceive
            | Lv2RequestKind::EventPortSend
            | Lv2RequestKind::EventFlagCreate
            | Lv2RequestKind::EventFlagDestroy
            | Lv2RequestKind::EventFlagWait
            | Lv2RequestKind::EventFlagSet
            | Lv2RequestKind::EventFlagClear
            | Lv2RequestKind::EventFlagTryWait
            | Lv2RequestKind::EventFlagCancel
            | Lv2RequestKind::EventFlagGet
            | Lv2RequestKind::CondCreate
            | Lv2RequestKind::CondDestroy
            | Lv2RequestKind::CondWait
            | Lv2RequestKind::CondSignal
            | Lv2RequestKind::CondSignalAll
            | Lv2RequestKind::CondSignalTo
            | Lv2RequestKind::LwMutexCreate
            | Lv2RequestKind::LwMutexDestroy
            | Lv2RequestKind::LwMutexLock
            | Lv2RequestKind::LwMutexUnlock
            | Lv2RequestKind::LwMutexTryLock => Modeled,

            // Memory: bump allocator with no reclamation.
            Lv2RequestKind::MemoryAllocate => PartialState,
            Lv2RequestKind::MemoryFree => AbiOnly,
            // Available derives from the allocator cursor; total is
            // the game-mode cap constant.
            Lv2RequestKind::MemoryGetUserMemorySize => PartialState,
            Lv2RequestKind::MemoryContainerCreate => AbiOnly,

            // Process.
            Lv2RequestKind::ProcessExit
            | Lv2RequestKind::ProcessGetPid
            | Lv2RequestKind::ProcessGetPpid
            | Lv2RequestKind::ProcessGetSdkVersion
            | Lv2RequestKind::ProcessIsStack => Modeled,
            // Real counts for modeled classes only.
            Lv2RequestKind::ProcessGetNumberOfObject => PartialState,
            // Serves the no-PARAM.SFO homebrew blob for every title.
            Lv2RequestKind::ProcessGetParamsfo => PartialState,
            Lv2RequestKind::ProcessGetPpuGuid => AbiOnly,
            // Unmodeled address nibbles answer CELL_EINVAL where real
            // LV2 consults sys_vm / mmapper state.
            Lv2RequestKind::ProcessIsSpuLockLineReservationAddress => PartialState,

            // Id-mint stubs with no backing object.
            Lv2RequestKind::TimerCreate
            | Lv2RequestKind::TimerDestroy
            | Lv2RequestKind::RwlockCreate
            | Lv2RequestKind::RwlockDestroy
            | Lv2RequestKind::EventPortCreate
            | Lv2RequestKind::EventPortDestroy => AbiOnly,

            // PPU threads.
            Lv2RequestKind::PpuThreadYield
            | Lv2RequestKind::PpuThreadExit
            | Lv2RequestKind::PpuThreadJoin => Modeled,
            // JOINABLE / INTERRUPT flags are logged, not modeled.
            Lv2RequestKind::PpuThreadCreate => PartialState,
            // Create-runs-immediately collapses the SUSPENDED state.
            Lv2RequestKind::PpuThreadStart => PartialState,

            // Read-only VFS.
            Lv2RequestKind::FsOpen
            | Lv2RequestKind::FsClose
            | Lv2RequestKind::FsRead
            | Lv2RequestKind::FsLseek
            | Lv2RequestKind::FsFstat
            | Lv2RequestKind::FsStat
            | Lv2RequestKind::FsOpendir
            | Lv2RequestKind::FsReaddir
            | Lv2RequestKind::FsClosedir => Modeled,
            // Real LV2 has writable mounts; the model refuses.
            Lv2RequestKind::FsWrite => PartialState,

            // RSX CPU-side model.
            Lv2RequestKind::SysRsxContextIomap => Modeled,
            Lv2RequestKind::SysRsxMemoryAllocate
            | Lv2RequestKind::SysRsxContextAllocate
            | Lv2RequestKind::SysRsxDeviceMap
            | Lv2RequestKind::SysRsxContextAttribute => PartialState,
            Lv2RequestKind::SysRsxMemoryFree | Lv2RequestKind::SysRsxContextFree => AbiOnly,

            Lv2RequestKind::SsAccessControlEngine => Modeled,
        })
    }
}

#[cfg(test)]
#[path = "tests/fidelity_tests.rs"]
mod tests;
