//! cellSpurs PS3 ABI: error codes, struct layouts, flag bits.
//!
//! Mirrors the layout of RPCS3's `rpcs3/Emu/Cell/Modules/cellSpurs.h`.
//! Behaviour (handlers, dispatch, allocation) lives in
//! `cellgov_core::hle::cellSpurs`; this module is data only.

/// `CellSpursCoreError` band (`0x8041_070x`). Returned by the
/// initialize / finalize / lifecycle handlers.
pub mod core_error {
    /// Invalid argument (null pointer, bad enum, bad combination).
    pub const INVAL: u32 = 0x8041_0702;
    /// Busy: the operation cannot complete because another caller
    /// already owns the resource.
    pub const BUSY: u32 = 0x8041_070A;
    /// Search failure: no such workload, no such handler.
    pub const SRCH: u32 = 0x8041_0705;
    /// Bad state (initialize on already-initialized SPURS, etc.).
    pub const STAT: u32 = 0x8041_070F;
    /// Misaligned argument pointer.
    pub const ALIGN: u32 = 0x8041_0710;
    /// Null argument where a pointer was required.
    pub const NULL_POINTER: u32 = 0x8041_0711;
}

/// `CellSpurs` control-block layout: byte offsets of every named field
/// the runtime reads or writes. Mirrors the field offsets in RPCS3's
/// `struct CellSpurs` (alignas 128, size 0x1000 SPURS1 / 0x2000 SPURS2).
/// Unnamed bytes stay zero from the post-init clear; only listed fields
/// are written.
pub mod layout {
    /// Required alignment of a `CellSpurs` allocation (`alignas(128)`).
    pub const ALIGN: u32 = 128;
    /// Size of a SPURS1 (legacy) control block.
    pub const SIZE_V1: u32 = 4096;
    /// Size of a SPURS2 control block.
    pub const SIZE_V2: u32 = 8192;
    /// Maximum guest-supplied name prefix length (without NUL).
    pub const NAME_MAX_LENGTH: u32 = 15;
    /// SPURS1 workload registry capacity.
    pub const MAX_WORKLOAD_V1: u32 = 16;
    /// SPURS2 workload registry capacity.
    pub const MAX_WORKLOAD_V2: u32 = 32;
    /// Maximum number of SPUs a `CellSpurs` instance can manage.
    pub const MAX_SPU: u32 = 8;
    /// Number of priority slots per workload (`priority[8]`).
    pub const MAX_PRIORITY: u32 = 16;
    /// `wklReadyCount1[16]` (atomic u8).
    pub const OFF_WKL_READY_COUNT_1: u32 = 0x00;
    /// `wklIdleSpuCountOrReadyCount2[16]` (atomic u8).
    pub const OFF_WKL_IDLE_SPU_COUNT_OR_RC2: u32 = 0x10;
    /// `wklMinContention[16]`.
    pub const OFF_WKL_MIN_CONTENTION: u32 = 0x40;
    /// `wklMaxContention[16]` (atomic u8).
    pub const OFF_WKL_MAX_CONTENTION: u32 = 0x50;
    /// `flags1` (u8).
    pub const OFF_FLAGS1: u32 = 0x74;
    /// `nSpus` (u8).
    pub const OFF_NSPUS: u32 = 0x76;
    /// `wklState1[16]` (atomic u8).
    pub const OFF_WKL_STATE_1: u32 = 0x80;
    /// `wklStatus1[16]`.
    pub const OFF_WKL_STATUS_1: u32 = 0x90;
    /// `wklEvent1[16]` (atomic u8).
    pub const OFF_WKL_EVENT_1: u32 = 0xA0;
    /// `wklEnabled` (atomic_be u32).
    pub const OFF_WKL_ENABLED: u32 = 0xB0;
    /// `wklMskB` (atomic_be u32) -- system-service available-module mask.
    pub const OFF_WKL_MSK_B: u32 = 0xB4;
    /// `sysSrvExitBarrier`.
    pub const OFF_SYS_SRV_MSG: u32 = 0xBC;
    /// `sysSrvMsgUpdateWorkload`.
    pub const OFF_SYS_SRV_MSG_UPDATE_WORKLOAD: u32 = 0xBD;
    /// `sysSrvPreemptWklId`.
    pub const OFF_SYS_SRV_PREEMPT_WKL_ID: u32 = 0xC0;
    /// `wklState2[16]`.
    pub const OFF_WKL_STATE_2: u32 = 0xD0;
    /// `wklStatus2[16]`.
    pub const OFF_WKL_STATUS_2: u32 = 0xE0;
    /// `wklEvent2[16]`.
    pub const OFF_WKL_EVENT_2: u32 = 0xF0;
    /// `wklInfo1[16]` (32 bytes each).
    pub const OFF_WKL_INFO_1: u32 = 0xB00;
    /// `wklInfoSysSrv` (32 bytes).
    pub const OFF_WKL_INFO_SYS_SRV: u32 = 0xD00;
    /// `wklInfo2[16]` (SPURS2 only).
    pub const OFF_WKL_INFO_2: u32 = 0x1000;
    /// `traceBuffer` (vm::bptr u64) -- 8-byte BE pointer.
    pub const OFF_TRACE_BUFFER: u32 = 0x900;
    /// `traceDataSize`.
    pub const OFF_TRACE_DATA_SIZE: u32 = 0x948;
    /// `traceMode`.
    pub const OFF_TRACE_MODE: u32 = 0x950;
    /// `ppu0` (PPU thread id).
    pub const OFF_PPU0: u32 = 0xD20;
    /// `ppu1` (PPU thread id).
    pub const OFF_PPU1: u32 = 0xD28;
    /// `spuTG` (SPU thread group id).
    pub const OFF_SPU_TG: u32 = 0xD30;
    /// `spus` (8 * `be_t<u32>`).
    pub const OFF_SPUS: u32 = 0xD34;
    /// `enableEH` (`atomic_be_t<u32>`).
    pub const OFF_ENABLE_EH: u32 = 0xD68;
    /// `exception` (`be_t<u32>`) -- set on SPURS exception.
    pub const OFF_EXCEPTION: u32 = 0xD6C;
    /// `flags` (u32).
    pub const OFF_FLAGS: u32 = 0xD80;
    /// `spuPriority` (u32).
    pub const OFF_SPU_PRIORITY: u32 = 0xD84;
    /// `ppuPriority` (u32).
    pub const OFF_PPU_PRIORITY: u32 = 0xD88;
    /// `prefix` (15-byte name).
    pub const OFF_PREFIX: u32 = 0xD8C;
    /// `prefixSize` (u8).
    pub const OFF_PREFIX_SIZE: u32 = 0xD9B;
    /// `revision` (u32).
    pub const OFF_REVISION: u32 = 0xDA0;
    /// `sdkVersion` (u32).
    pub const OFF_SDK_VERSION: u32 = 0xDA4;
    /// `spuPortBits` (`atomic_be_t<u64>`).
    pub const OFF_SPU_PORT_BITS: u32 = 0xDA8;
    /// `eventPortMux` substruct (128 bytes).
    pub const OFF_EVENT_PORT_MUX: u32 = 0xF00;
    /// `globalSpuExceptionHandler` (`atomic_be_t<u64>`).
    pub const OFF_GLOBAL_EXCEPTION_HANDLER: u32 = 0xF80;
    /// `globalSpuExceptionHandlerArgs` (`be_t<u64>`).
    pub const OFF_GLOBAL_EXCEPTION_HANDLER_ARGS: u32 = 0xF88;
}

/// `CellSpursAttribute` layout (size 512, `alignas(8)`).
pub mod attribute_layout {
    /// `sizeof(CellSpursAttribute)`.
    pub const SIZE: u32 = 512;
    /// Required alignment.
    pub const ALIGN: u32 = 8;
    /// `revision` (u32).
    pub const OFF_REVISION: u32 = 0x00;
    /// `sdkVersion` (u32).
    pub const OFF_SDK_VERSION: u32 = 0x04;
    /// `nSpus` (u32).
    pub const OFF_NSPUS: u32 = 0x08;
    /// `spuPriority` (u32).
    pub const OFF_SPU_PRIORITY: u32 = 0x0C;
    /// `ppuPriority` (u32).
    pub const OFF_PPU_PRIORITY: u32 = 0x10;
    /// `exitIfNoWork` (u32).
    pub const OFF_EXIT_IF_NO_WORK: u32 = 0x14;
    /// `prefix` (15-byte name).
    pub const OFF_PREFIX: u32 = 0x15;
    /// `prefixSize` (u32).
    pub const OFF_PREFIX_SIZE: u32 = 0x24;
    /// `flags` (u32).
    pub const OFF_FLAGS: u32 = 0x28;
    /// `container` (u32).
    pub const OFF_CONTAINER: u32 = 0x2C;
    /// `swlPriority` (8 bytes).
    pub const OFF_SWL_PRIORITY: u32 = 0x38;
    /// `swlMaxSpu` (u32).
    pub const OFF_SWL_MAX_SPU: u32 = 0x40;
    /// `swlIsPreem` (u32).
    pub const OFF_SWL_IS_PREEM: u32 = 0x44;
}

/// `CellSpursWorkloadAttribute` layout (`alignas(8)`).
pub mod workload_attribute_layout {
    /// Required alignment.
    pub const ALIGN: u32 = 8;
    /// `revision` (u32).
    pub const OFF_REVISION: u32 = 0x00;
    /// `pm` (policy-module address, u64).
    pub const OFF_PM: u32 = 0x08;
    /// `size` (u32).
    pub const OFF_SIZE: u32 = 0x0C;
    /// `data` (u64).
    pub const OFF_DATA: u32 = 0x10;
    /// `priority` (8 bytes).
    pub const OFF_PRIORITY: u32 = 0x18;
    /// `minContention` (u32).
    pub const OFF_MIN_CONTENTION: u32 = 0x20;
    /// `maxContention` (u32).
    pub const OFF_MAX_CONTENTION: u32 = 0x24;
}

/// `WorkloadInfo` entry layout (32 bytes per entry inside `wklInfo1` /
/// `wklInfo2`). Format: `addr u64 +0x00, arg u64 +0x08, size u32 +0x10,
/// uniqueId u8 +0x14, priority[8] +0x18`.
pub mod workload_info_layout {
    /// Bytes per `WorkloadInfo` entry.
    pub const SIZE: u32 = 32;
    /// `addr` (policy-module address, u64).
    pub const OFF_ADDR: u32 = 0x00;
    /// `arg` (u64).
    pub const OFF_ARG: u32 = 0x08;
    /// `size` (u32).
    pub const OFF_SIZE: u32 = 0x10;
    /// `uniqueId` (u8).
    pub const OFF_UNIQUE_ID: u32 = 0x14;
    /// `priority[8]`.
    pub const OFF_PRIORITY: u32 = 0x18;
}

/// `CellSpursInfo` layout (size 280 per `CHECK_SIZE(CellSpursInfo, 280)`).
pub mod info_layout {
    /// `sizeof(CellSpursInfo)`.
    pub const SIZE: u32 = 280;
    /// `nSpus` (u32).
    pub const OFF_NSPUS: u32 = 0x00;
    /// `spuThreadGroupPriority` (u32).
    pub const OFF_SPU_THREAD_GROUP_PRIORITY: u32 = 0x04;
    /// `ppuThreadPriority` (u32).
    pub const OFF_PPU_THREAD_PRIORITY: u32 = 0x08;
    /// `exitIfNoWork` (u32).
    pub const OFF_EXIT_IF_NO_WORK: u32 = 0x0C;
    /// `spurs2` (u8 boolean).
    pub const OFF_SPURS2: u32 = 0x0D;
    /// `traceBuffer` (`vm::bptr<void>`) -- 4-byte BE pointer.
    pub const OFF_TRACE_BUFFER: u32 = 0x10;
    /// `traceBufferSize` (u32).
    pub const OFF_TRACE_BUFFER_SIZE: u32 = 0x18;
    /// `traceMode` (u32).
    pub const OFF_TRACE_MODE: u32 = 0x20;
    /// `spuThreadGroup` (u32).
    pub const OFF_SPU_THREAD_GROUP: u32 = 0x24;
    /// `spuThreads[8]` (8 * `be_t<u32>`).
    pub const OFF_SPU_THREADS: u32 = 0x28;
    /// `spursHandlerThread0` (u64).
    pub const OFF_SPURS_HANDLER_THREAD_0: u32 = 0x48;
    /// `spursHandlerThread1` (u64).
    pub const OFF_SPURS_HANDLER_THREAD_1: u32 = 0x50;
    /// `namePrefix[16]`.
    pub const OFF_NAME_PREFIX: u32 = 0x58;
    /// `namePrefixLength` (u32).
    pub const OFF_NAME_PREFIX_LENGTH: u32 = 0x68;
    /// `deadlineMissCounter` (u32).
    pub const OFF_DEADLINE_MISS_COUNTER: u32 = 0x6C;
    /// `deadlineMeetCounter` (u32).
    pub const OFF_DEADLINE_MEET_COUNTER: u32 = 0x70;
}

/// `EventPortMux` sub-block offsets (relative to `layout::OFF_EVENT_PORT_MUX`).
pub mod event_port_mux_layout {
    /// `spuPort` (`be_t<u32>`).
    pub const OFF_SPU_PORT: u32 = 0x04;
    /// `eventPort` (`be_t<u64>`).
    pub const OFF_EVENT_PORT: u32 = 0x10;
}

/// `wklState1[]` / `wklState2[]` slot values. State 0 (`NON_EXISTENT`)
/// is the post-zero default and never explicitly named.
pub mod wkl_state {
    /// Slot is allocated but the workload's policy module hasn't loaded yet.
    pub const PREPARING: u8 = 1;
    /// Slot is dispatchable.
    pub const RUNNABLE: u8 = 2;
    /// Workload has been asked to terminate; SPU side draining.
    pub const SHUTTING_DOWN: u8 = 3;
    /// Drain complete; the slot is reusable on next `add_workload`.
    pub const REMOVABLE: u8 = 4;
}

/// `CellSpursAttribute::flags` bits (`SAF_*`).
pub mod saf {
    /// No flags.
    pub const NONE: u32 = 0x0;
    /// Workers exit when no workload is dispatchable.
    pub const EXIT_IF_NO_WORK: u32 = 0x1;
    /// Caller is initialising a SPURS2 control block.
    pub const SECOND_VERSION: u32 = 0x4;
}

/// System-service workload sentinels (filled by `cellSpursInitialize`).
pub mod sys_srv {
    /// `wklInfoSysSrv.addr` sentinel value set by initialize.
    pub const IMG_ADDR: u32 = 0x100;
    /// System-service workload byte size.
    pub const WORKLOAD_SIZE: u32 = 0x2200;
}

/// `CellSpursPolicyModuleError` band (`0x8041_080x`). Returned by the
/// workload registry, ready-count, contention, and priority handlers.
pub mod policy_module_error {
    /// Resource exhausted (workload registry full, etc.).
    pub const AGAIN: u32 = 0x8041_0801;
    /// Invalid argument.
    pub const INVAL: u32 = 0x8041_0802;
    /// Search failure.
    pub const SRCH: u32 = 0x8041_0805;
    /// Fault during the workload-registry operation.
    pub const FAULT: u32 = 0x8041_080D;
    /// Bad workload state (operation on a workload not in the
    /// expected state).
    pub const STAT: u32 = 0x8041_080F;
    /// Misaligned argument pointer.
    pub const ALIGN: u32 = 0x8041_0810;
    /// Null argument where a pointer was required.
    pub const NULL_POINTER: u32 = 0x8041_0811;
}

// Compile-time tripwires for cross-field offset relationships. A
// rename or renumber that breaks one of these fails the build before
// any silent data corruption (writing the wrong field, indexing past
// a region limit) can ship.
#[allow(dead_code)]
const _COMPILE_TIME_OFFSETS: () = {
    // CellSpurs region bounds.
    assert!(layout::OFF_PPU0 < layout::SIZE_V1);
    assert!(layout::OFF_PPU0 < layout::SIZE_V2);
    assert!(layout::OFF_REVISION < layout::SIZE_V1);
    assert!(layout::OFF_EXCEPTION < layout::SIZE_V1);
    // exception (be_t<u32>) is immediately followed by sys_spu_image
    // spuImg at 0xD70. Tightening layout::OFF_EXCEPTION to enableEH
    // (0xD68) by mistake would still pass the size check; this pins
    // the gap so the misread fails compile.
    const SPU_IMG_START: u32 = 0xD70;
    assert!(layout::OFF_EXCEPTION + 4 == SPU_IMG_START);
    assert!(SPU_IMG_START < layout::SIZE_V1);
    assert!(layout::OFF_WKL_MSK_B + 4 == 0xB8);
    assert!(layout::OFF_PREFIX + 15 == layout::OFF_PREFIX_SIZE);
    // prefixSize is a u8; unk5 (u32) follows at 0xD9C. Renumbering
    // prefixSize into a 4-byte field would clobber unk5.
    assert!(layout::OFF_PREFIX_SIZE + 1 == 0xD9C);
    assert!(layout::OFF_REVISION + 4 == layout::OFF_SDK_VERSION);
    assert!(layout::OFF_FLAGS + 4 == layout::OFF_SPU_PRIORITY);
    assert!(layout::OFF_SPU_PRIORITY + 4 == layout::OFF_PPU_PRIORITY);
    // wklInfo2[16] must fit inside the SPURS2 8 KiB region but live
    // past the SPURS1 4 KiB limit (the SPURS2-only bank).
    assert!(layout::OFF_WKL_INFO_2 + 16 * workload_info_layout::SIZE <= layout::SIZE_V2);
    assert!(layout::OFF_WKL_INFO_2 >= layout::SIZE_V1);
    assert!(attribute_layout::OFF_PREFIX + 15 == attribute_layout::OFF_PREFIX_SIZE);
    assert!(attribute_layout::OFF_REVISION + 4 == attribute_layout::OFF_SDK_VERSION);
    assert!(attribute_layout::OFF_NSPUS + 4 == attribute_layout::OFF_SPU_PRIORITY);
    assert!(attribute_layout::OFF_SPU_PRIORITY + 4 == attribute_layout::OFF_PPU_PRIORITY);
    assert!(attribute_layout::OFF_PPU_PRIORITY + 4 == attribute_layout::OFF_EXIT_IF_NO_WORK);
    assert!(attribute_layout::OFF_FLAGS + 4 == attribute_layout::OFF_CONTAINER);
    assert!(attribute_layout::OFF_SWL_PRIORITY + 8 == attribute_layout::OFF_SWL_MAX_SPU);
    assert!(attribute_layout::OFF_SWL_MAX_SPU + 4 == attribute_layout::OFF_SWL_IS_PREEM);
    // EventPortMux substruct (128 bytes) and the deepest written
    // field inside it (eventPort at +0x10) must stay in SPURS1 range.
    assert!(layout::OFF_EVENT_PORT_MUX + 128 <= layout::SIZE_V1);
    assert!(
        layout::OFF_EVENT_PORT_MUX + event_port_mux_layout::OFF_EVENT_PORT + 8 <= layout::SIZE_V1
    );
    // globalSpuExceptionHandler / args at 0xF80 / 0xF88.
    assert!(layout::OFF_GLOBAL_EXCEPTION_HANDLER + 8 == layout::OFF_GLOBAL_EXCEPTION_HANDLER_ARGS);
    assert!(layout::OFF_GLOBAL_EXCEPTION_HANDLER_ARGS + 8 <= layout::SIZE_V1);
    // CellSpursInfo: padding[164] starts at 0x74, so total = 280.
    assert!(info_layout::OFF_NAME_PREFIX + 16 == info_layout::OFF_NAME_PREFIX_LENGTH);
    assert!(info_layout::OFF_NAME_PREFIX_LENGTH + 4 == info_layout::OFF_DEADLINE_MISS_COUNTER);
    assert!(info_layout::OFF_DEADLINE_MISS_COUNTER + 4 == info_layout::OFF_DEADLINE_MEET_COUNTER);
    assert!(info_layout::OFF_SPU_THREADS + 32 == info_layout::OFF_SPURS_HANDLER_THREAD_0);
    assert!(info_layout::OFF_SPURS_HANDLER_THREAD_0 + 8 == info_layout::OFF_SPURS_HANDLER_THREAD_1);
    assert!(info_layout::OFF_DEADLINE_MEET_COUNTER + 4 + 164 == info_layout::SIZE);
};
