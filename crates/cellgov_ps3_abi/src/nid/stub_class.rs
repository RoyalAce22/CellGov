//! Diagnostic stub classification for the curated NIDs.

use super::{cell_gcm_sys, cell_save_data, cell_spurs, cell_sysutil, sys_fs, sys_prx_for_user};

/// HLE-stub safety class. The class governs whether returning 0 from a
/// stubbed import is safe for short triage runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StubClass {
    /// Returning 0 leaves correctness intact for the duration of a
    /// triage run (cosmetic-output, GUI-only, or genuine no-ops).
    NoopSafe,
    /// The function maintains kernel/runtime state the program reads
    /// back later (heap handles, mutex IDs, thread IDs, time values).
    /// A stub must mint stable values, not return 0.
    Stateful,
    /// The function returns a resource the program will dereference.
    /// Returning 0 (null pointer / invalid handle) faults on next use.
    UnsafeToStub,
}

impl StubClass {
    /// Hyphenated label used in `dump-imports` inventory tables.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NoopSafe => "noop-safe",
            Self::Stateful => "stateful",
            Self::UnsafeToStub => "unsafe-to-stub",
        }
    }
}

/// Diagnostic class for `nid`. Used by `cellgov dev prx-imports`
/// to label unresolved-or-zero-bound PRX imports as `noop-safe` /
/// `stateful` / `unsafe-to-stub`. Reviewed NIDs are classified
/// explicitly; everything else defaults to `NoopSafe`. The default
/// is presumptive, not verified -- a NID we have not reviewed could
/// be `Stateful` or `UnsafeToStub`. NIDs grouped under any module's
/// `CLASSIFIED_NIDS` slice must classify explicitly (enforced by
/// `nid::tests::every_classified_nid_has_explicit_arm`).
///
/// For "is this NID explicitly classified or did it fall through the
/// default" use [`stub_classification_explicit`]; this wrapper is the
/// total form for consumers (e.g. `dev prx-imports`) that just want
/// a label.
///
/// The project does not substitute Rust
/// handlers for PRX libraries; PRX-side surfaces are bound via
/// `cellgov_ppu::prx_loader::patch_imports_against` to firmware
/// OPDs at boot.
pub fn stub_classification(nid: u32) -> StubClass {
    stub_classification_explicit(nid).unwrap_or(StubClass::NoopSafe)
}

/// Like [`stub_classification`] but returns `None` for NIDs that
/// fall through to the default `NoopSafe` catch-all. Used by the
/// `every_classified_nid_has_explicit_arm` regression to enforce
/// that every NID listed in any module's `CLASSIFIED_NIDS` slice is
/// reviewed rather than presumptively-NoopSafe. A legitimately-
/// NoopSafe NID (e.g. `_sys_free` -- "leak is OK") still has its
/// own explicit arm returning `Some(StubClass::NoopSafe)`.
pub fn stub_classification_explicit(nid: u32) -> Option<StubClass> {
    use cell_gcm_sys as gcm;
    use cell_save_data as savedata;
    use cell_spurs as spurs;
    use cell_sysutil as sysutil;
    use sys_fs as fs;
    use sys_prx_for_user as sys;
    match nid {
        // RSX / GCM library: every implemented surface mutates or reads
        // driver state, so a 0-returning stub corrupts later GCM calls.
        gcm::GET_TILED_PITCH_SIZE
        | gcm::INIT_BODY
        | gcm::GET_CONTROL_REGISTER
        | gcm::GET_CONFIGURATION
        | gcm::GET_LABEL_ADDRESS
        | gcm::ADDRESS_TO_OFFSET
        | gcm::SET_FLIP_HANDLER => Some(StubClass::Stateful),
        // sysPrxForUser TLS / memory primitives.
        sys::INITIALIZE_TLS | sys::MEMSET | sys::PROCESS_EXIT => Some(StubClass::Stateful),
        sys::MALLOC => Some(StubClass::UnsafeToStub),
        sys::FREE => Some(StubClass::NoopSafe), // leak is OK
        // User-mode heap allocator.
        sys::HEAP_CREATE_HEAP => Some(StubClass::Stateful),
        sys::HEAP_DELETE_HEAP => Some(StubClass::Stateful),
        sys::HEAP_MALLOC | sys::HEAP_MEMALIGN => Some(StubClass::UnsafeToStub),
        // HEAP_FREE: freeing a pointer the title later reuses is a
        // use-after-free; same class as HEAP_MALLOC / HEAP_MEMALIGN.
        sys::HEAP_FREE => Some(StubClass::UnsafeToStub),
        // Lightweight mutex family: every entry mutates sync state.
        sys::LWMUTEX_CREATE
        | sys::LWMUTEX_LOCK
        | sys::LWMUTEX_DESTROY
        | sys::LWMUTEX_UNLOCK
        | sys::LWMUTEX_TRYLOCK => Some(StubClass::Stateful),
        // Lightweight cond family: count-only stubs today, but the
        // count itself is observable state.
        sys::LWCOND_CREATE | sys::LWCOND_DESTROY => Some(StubClass::Stateful),
        // Time / thread / process queries.
        sys::TIME_GET_SYSTEM_TIME
        | sys::PPU_THREAD_GET_ID
        | sys::PPU_THREAD_CREATE
        | sys::PPU_THREAD_EXIT
        | sys::PROCESS_IS_STACK
        | sys::PRX_EXITSPAWN_WITH_LEVEL => Some(StubClass::Stateful),
        // cellSysutil video-out queries.
        sysutil::VIDEO_OUT_GET_STATE | sysutil::VIDEO_OUT_GET_RESOLUTION => {
            Some(StubClass::Stateful)
        }
        // cellSpurs initialize family.
        spurs::ATTRIBUTE_INITIALIZE
        | spurs::INITIALIZE
        | spurs::INITIALIZE_WITH_ATTRIBUTE
        | spurs::INITIALIZE_WITH_ATTRIBUTE2
        | spurs::FINALIZE => Some(StubClass::Stateful),
        // cellSpurs workload registry.
        spurs::WORKLOAD_ATTRIBUTE_INITIALIZE
        | spurs::ADD_WORKLOAD
        | spurs::ADD_WORKLOAD_WITH_ATTRIBUTE
        | spurs::SHUTDOWN_WORKLOAD
        | spurs::WAIT_FOR_WORKLOAD_SHUTDOWN => Some(StubClass::Stateful),
        // cellSpurs ready-count, contention, idle-spu, priority controls.
        spurs::READY_COUNT_STORE
        | spurs::READY_COUNT_ADD
        | spurs::READY_COUNT_SWAP
        | spurs::READY_COUNT_COMPARE_AND_SWAP
        | spurs::REQUEST_IDLE_SPU
        | spurs::SET_MAX_CONTENTION
        | spurs::SET_PRIORITIES
        | spurs::SET_PRIORITY => Some(StubClass::Stateful),
        // cellSpurs info getter + exception handler registration.
        spurs::GET_INFO
        | spurs::ATTACH_LV2_EVENT_QUEUE
        | spurs::DETACH_LV2_EVENT_QUEUE
        | spurs::SET_EXCEPTION_EVENT_HANDLER
        | spurs::UNSET_EXCEPTION_EVENT_HANDLER
        | spurs::SET_GLOBAL_EXCEPTION_EVENT_HANDLER
        | spurs::UNSET_GLOBAL_EXCEPTION_EVENT_HANDLER
        | spurs::ENABLE_EXCEPTION_EVENT_HANDLER => Some(StubClass::Stateful),
        // sys_fs HLE wrappers: every entry forwards to the LV2
        // sys_fs_* surface and mutates fd-table / blob state. A
        // 0-returning stub would let the title proceed with an
        // uninitialized fd or skipped read, corrupting downstream
        // state.
        fs::OPEN
        | fs::READ
        | fs::CLOSE
        | fs::LSEEK
        | fs::FSTAT
        | fs::STAT
        | fs::OPENDIR
        | fs::READDIR
        | fs::CLOSEDIR => Some(StubClass::Stateful),
        // cellSaveData autoload: the stub returns the no-save
        // sentinel (sign-extended NODATA) before any title callback
        // fires. A blanket 0-return would hand back CELL_OK and let
        // the title proceed as if save data were loaded; explicit
        // Stateful classification flags this as "non-noop, must
        // implement" rather than "leak-safe stub".
        savedata::AUTO_LOAD | savedata::AUTO_LOAD_2 => Some(StubClass::Stateful),
        _ => Some(StubClass::NoopSafe),
    }
}
