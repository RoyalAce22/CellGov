//! Triage verdict for a PRX import that `dev prx-imports` lists: whether
//! a stub that returns 0 in its place is safe for a short run.
//!
//! The verdicts are CellGov's own review notes over the curated NIDs
//! in `cellgov_ps3_abi::nid`. CellGov substitutes no Rust handler for
//! a PRX library; the loader binds PRX-side imports to firmware OPDs
//! at boot. The label only tells the reader of the inventory table
//! what an unbound import would cost.

use cellgov_ps3_abi::nid::{
    cell_gcm_sys as gcm, cell_save_data as savedata, cell_spurs as spurs, cell_sysutil as sysutil,
    sys_fs as fs, sys_prx_for_user as sys,
};

/// What a 0-returning stub does to the program that called it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StubClass {
    /// Correctness holds for the duration of a triage run
    /// (cosmetic output, GUI-only, or a genuine no-op).
    NoopSafe,
    /// The function maintains state the program reads back later
    /// (heap handles, mutex ids, thread ids, time values), so a stub
    /// has to mint stable values.
    Stateful,
    /// The function returns a resource the program dereferences;
    /// a 0 (null pointer or invalid handle) faults on next use.
    UnsafeToStub,
}

impl StubClass {
    /// Hyphenated label for the inventory table's `Class` column.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NoopSafe => "noop-safe",
            Self::Stateful => "stateful",
            Self::UnsafeToStub => "unsafe-to-stub",
        }
    }
}

/// Verdict for `nid`, or `NoopSafe` when no review recorded one.
///
/// A `NoopSafe` answer can be that presumption, and an unreviewed NID
/// may well be `Stateful` or `UnsafeToStub`.
/// [`reviewed_stub_classification`] tells the two apart.
pub fn stub_classification(nid: u32) -> StubClass {
    reviewed_stub_classification(nid).unwrap_or(StubClass::NoopSafe)
}

/// Verdict a review recorded for `nid`, or `None` when none did.
///
/// A NID a review found `NoopSafe` (`_sys_free`: the leak is fine)
/// still answers `Some`.
pub fn reviewed_stub_classification(nid: u32) -> Option<StubClass> {
    let class = match nid {
        // RSX / GCM library: every implemented surface mutates or reads
        // driver state, so a 0-returning stub corrupts later GCM calls.
        gcm::GET_TILED_PITCH_SIZE
        | gcm::INIT_BODY
        | gcm::GET_CONTROL_REGISTER
        | gcm::GET_CONFIGURATION
        | gcm::GET_LABEL_ADDRESS
        | gcm::ADDRESS_TO_OFFSET
        | gcm::SET_FLIP_HANDLER => StubClass::Stateful,
        // sysPrxForUser TLS / memory primitives.
        sys::INITIALIZE_TLS | sys::MEMSET | sys::PROCESS_EXIT => StubClass::Stateful,
        sys::MALLOC => StubClass::UnsafeToStub,
        sys::FREE => StubClass::NoopSafe, // leak is OK
        // User-mode heap allocator.
        sys::HEAP_CREATE_HEAP => StubClass::Stateful,
        sys::HEAP_DELETE_HEAP => StubClass::Stateful,
        sys::HEAP_MALLOC | sys::HEAP_MEMALIGN => StubClass::UnsafeToStub,
        // HEAP_FREE: freeing a pointer the title later reuses is a
        // use-after-free; same class as HEAP_MALLOC / HEAP_MEMALIGN.
        sys::HEAP_FREE => StubClass::UnsafeToStub,
        // Lightweight mutex family: every entry mutates sync state.
        sys::LWMUTEX_CREATE
        | sys::LWMUTEX_LOCK
        | sys::LWMUTEX_DESTROY
        | sys::LWMUTEX_UNLOCK
        | sys::LWMUTEX_TRYLOCK => StubClass::Stateful,
        // Lightweight cond family: the create/destroy count is
        // observable state.
        sys::LWCOND_CREATE | sys::LWCOND_DESTROY => StubClass::Stateful,
        // Time / thread / process queries.
        sys::TIME_GET_SYSTEM_TIME
        | sys::PPU_THREAD_GET_ID
        | sys::PPU_THREAD_CREATE
        | sys::PPU_THREAD_EXIT
        | sys::PROCESS_IS_STACK
        | sys::PRX_EXITSPAWN_WITH_LEVEL => StubClass::Stateful,
        // cellSysutil video-out queries.
        sysutil::VIDEO_OUT_GET_STATE | sysutil::VIDEO_OUT_GET_RESOLUTION => StubClass::Stateful,
        // cellSpurs initialize family.
        spurs::ATTRIBUTE_INITIALIZE
        | spurs::INITIALIZE
        | spurs::INITIALIZE_WITH_ATTRIBUTE
        | spurs::INITIALIZE_WITH_ATTRIBUTE2
        | spurs::FINALIZE => StubClass::Stateful,
        // cellSpurs workload registry.
        spurs::WORKLOAD_ATTRIBUTE_INITIALIZE
        | spurs::ADD_WORKLOAD
        | spurs::ADD_WORKLOAD_WITH_ATTRIBUTE
        | spurs::SHUTDOWN_WORKLOAD
        | spurs::WAIT_FOR_WORKLOAD_SHUTDOWN => StubClass::Stateful,
        // cellSpurs ready-count, contention, idle-spu, priority controls.
        spurs::READY_COUNT_STORE
        | spurs::READY_COUNT_ADD
        | spurs::READY_COUNT_SWAP
        | spurs::READY_COUNT_COMPARE_AND_SWAP
        | spurs::REQUEST_IDLE_SPU
        | spurs::SET_MAX_CONTENTION
        | spurs::SET_PRIORITIES
        | spurs::SET_PRIORITY => StubClass::Stateful,
        // cellSpurs info getter + exception handler registration.
        spurs::GET_INFO
        | spurs::ATTACH_LV2_EVENT_QUEUE
        | spurs::DETACH_LV2_EVENT_QUEUE
        | spurs::SET_EXCEPTION_EVENT_HANDLER
        | spurs::UNSET_EXCEPTION_EVENT_HANDLER
        | spurs::SET_GLOBAL_EXCEPTION_EVENT_HANDLER
        | spurs::UNSET_GLOBAL_EXCEPTION_EVENT_HANDLER
        | spurs::ENABLE_EXCEPTION_EVENT_HANDLER => StubClass::Stateful,
        // sys_fs HLE wrappers: every entry forwards to the LV2
        // sys_fs_* surface and mutates fd-table / blob state. A
        // 0-returning stub would let the title proceed with an
        // uninitialized fd or a skipped read and corrupt downstream
        // state.
        fs::OPEN
        | fs::READ
        | fs::CLOSE
        | fs::LSEEK
        | fs::FSTAT
        | fs::STAT
        | fs::OPENDIR
        | fs::READDIR
        | fs::CLOSEDIR => StubClass::Stateful,
        // cellSaveData autoload: the stub returns the no-save
        // sentinel (sign-extended NODATA) before any title callback
        // fires. A blanket 0 return would hand back CELL_OK and let
        // the title proceed as if it loaded save data.
        savedata::AUTO_LOAD | savedata::AUTO_LOAD_2 => StubClass::Stateful,
        _ => return None,
    };
    Some(class)
}

#[cfg(test)]
#[path = "tests/stub_class_tests.rs"]
mod tests;
