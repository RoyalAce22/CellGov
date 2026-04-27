//! cellSpurs PPU-side HLE handlers.
//!
//! The `CellSpurs` control block lives in guest memory; this module
//! owns the field-offset constants and the per-NID handlers that read
//! and write that block. SPU-side workload dispatch, policy-module
//! DMA, and taskset execution are out of scope.
//!
//! ## Alignment invariants
//!
//! `CellSpurs` is `alignas(128)`; `CellSpursAttribute` is `alignas(8)`.
//! Every entrypoint rejects misaligned pointers with the matching
//! `_ALIGN` error code rather than silently writing through a
//! misaligned address.

use cellgov_event::UnitId;

use crate::hle::context::{HleContext, HleReadError, RuntimeHleAdapter};
use crate::runtime::Runtime;

pub(crate) const NID_CELL_SPURS_ATTRIBUTE_INITIALIZE: u32 = 0x95180230;
pub(crate) const NID_CELL_SPURS_INITIALIZE: u32 = 0xacfc8dbc;
pub(crate) const NID_CELL_SPURS_INITIALIZE_WITH_ATTRIBUTE: u32 = 0xaa6269a8;
pub(crate) const NID_CELL_SPURS_INITIALIZE_WITH_ATTRIBUTE2: u32 = 0x30aa96c4;
pub(crate) const NID_CELL_SPURS_FINALIZE: u32 = 0xca4c4600;

pub(crate) const NID_CELL_SPURS_ADD_WORKLOAD: u32 = 0x69726aa2;
pub(crate) const NID_CELL_SPURS_ADD_WORKLOAD_WITH_ATTRIBUTE: u32 = 0xc0158d8b;
pub(crate) const NID_CELL_SPURS_WORKLOAD_ATTRIBUTE_INITIALIZE: u32 = 0xefeb2679;
pub(crate) const NID_CELL_SPURS_SHUTDOWN_WORKLOAD: u32 = 0x98d5b343;
pub(crate) const NID_CELL_SPURS_WAIT_FOR_WORKLOAD_SHUTDOWN: u32 = 0x5fd43fe4;

pub(crate) const NID_CELL_SPURS_READY_COUNT_STORE: u32 = 0xf843818d;
pub(crate) const NID_CELL_SPURS_READY_COUNT_ADD: u32 = 0x75211196;
pub(crate) const NID_CELL_SPURS_READY_COUNT_SWAP: u32 = 0x49a3426d;
pub(crate) const NID_CELL_SPURS_READY_COUNT_COMPARE_AND_SWAP: u32 = 0xf1d3552d;
pub(crate) const NID_CELL_SPURS_REQUEST_IDLE_SPU: u32 = 0x182d9890;
pub(crate) const NID_CELL_SPURS_SET_MAX_CONTENTION: u32 = 0x84d2f6d5;
pub(crate) const NID_CELL_SPURS_SET_PRIORITIES: u32 = 0x80a29e27;
pub(crate) const NID_CELL_SPURS_SET_PRIORITY: u32 = 0xb52e1bda;

pub(crate) const NID_CELL_SPURS_GET_INFO: u32 = 0x1f402f8f;
pub(crate) const NID_CELL_SPURS_ATTACH_LV2_EVENT_QUEUE: u32 = 0xb9bc6207;
pub(crate) const NID_CELL_SPURS_DETACH_LV2_EVENT_QUEUE: u32 = 0x4e66d483;
pub(crate) const NID_CELL_SPURS_SET_EXCEPTION_EVENT_HANDLER: u32 = 0xd2e23fa9;
pub(crate) const NID_CELL_SPURS_UNSET_EXCEPTION_EVENT_HANDLER: u32 = 0x4c75deb8;
pub(crate) const NID_CELL_SPURS_SET_GLOBAL_EXCEPTION_EVENT_HANDLER: u32 = 0x7517724a;
pub(crate) const NID_CELL_SPURS_UNSET_GLOBAL_EXCEPTION_EVENT_HANDLER: u32 = 0x861237f8;
pub(crate) const NID_CELL_SPURS_ENABLE_EXCEPTION_EVENT_HANDLER: u32 = 0x32b94add;

/// Every NID this module claims; the dispatcher's match arms must stay
/// in sync with this list (enforced by the canary tests below).
#[cfg(test)]
pub(crate) const OWNED_NIDS: &[u32] = &[
    NID_CELL_SPURS_ATTRIBUTE_INITIALIZE,
    NID_CELL_SPURS_INITIALIZE,
    NID_CELL_SPURS_INITIALIZE_WITH_ATTRIBUTE,
    NID_CELL_SPURS_INITIALIZE_WITH_ATTRIBUTE2,
    NID_CELL_SPURS_FINALIZE,
    NID_CELL_SPURS_ADD_WORKLOAD,
    NID_CELL_SPURS_ADD_WORKLOAD_WITH_ATTRIBUTE,
    NID_CELL_SPURS_WORKLOAD_ATTRIBUTE_INITIALIZE,
    NID_CELL_SPURS_SHUTDOWN_WORKLOAD,
    NID_CELL_SPURS_WAIT_FOR_WORKLOAD_SHUTDOWN,
    NID_CELL_SPURS_READY_COUNT_STORE,
    NID_CELL_SPURS_READY_COUNT_ADD,
    NID_CELL_SPURS_READY_COUNT_SWAP,
    NID_CELL_SPURS_READY_COUNT_COMPARE_AND_SWAP,
    NID_CELL_SPURS_REQUEST_IDLE_SPU,
    NID_CELL_SPURS_SET_MAX_CONTENTION,
    NID_CELL_SPURS_SET_PRIORITIES,
    NID_CELL_SPURS_SET_PRIORITY,
    NID_CELL_SPURS_GET_INFO,
    NID_CELL_SPURS_ATTACH_LV2_EVENT_QUEUE,
    NID_CELL_SPURS_DETACH_LV2_EVENT_QUEUE,
    NID_CELL_SPURS_SET_EXCEPTION_EVENT_HANDLER,
    NID_CELL_SPURS_UNSET_EXCEPTION_EVENT_HANDLER,
    NID_CELL_SPURS_SET_GLOBAL_EXCEPTION_EVENT_HANDLER,
    NID_CELL_SPURS_UNSET_GLOBAL_EXCEPTION_EVENT_HANDLER,
    NID_CELL_SPURS_ENABLE_EXCEPTION_EVENT_HANDLER,
];

// CELL_SPURS_CORE_ERROR_* (0x8041070x band).

const CELL_SPURS_CORE_ERROR_INVAL: u32 = 0x8041_0702;
const CELL_SPURS_CORE_ERROR_BUSY: u32 = 0x8041_070A;
const CELL_SPURS_CORE_ERROR_SRCH: u32 = 0x8041_0705;
const CELL_SPURS_CORE_ERROR_STAT: u32 = 0x8041_070F;
const CELL_SPURS_CORE_ERROR_ALIGN: u32 = 0x8041_0710;
const CELL_SPURS_CORE_ERROR_NULL_POINTER: u32 = 0x8041_0711;

// CELL_SPURS_POLICY_MODULE_ERROR_* (0x8041080x band).

const CELL_SPURS_POLICY_MODULE_ERROR_AGAIN: u32 = 0x8041_0801;
const CELL_SPURS_POLICY_MODULE_ERROR_INVAL: u32 = 0x8041_0802;
const CELL_SPURS_POLICY_MODULE_ERROR_SRCH: u32 = 0x8041_0805;
const CELL_SPURS_POLICY_MODULE_ERROR_FAULT: u32 = 0x8041_080D;
const CELL_SPURS_POLICY_MODULE_ERROR_STAT: u32 = 0x8041_080F;
const CELL_SPURS_POLICY_MODULE_ERROR_ALIGN: u32 = 0x8041_0810;
const CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER: u32 = 0x8041_0811;

// CellSpursAttribute layout (size 512, alignas 8).

const CELL_SPURS_ATTRIBUTE_SIZE: u32 = 512;
const CELL_SPURS_ATTRIBUTE_ALIGN: u32 = 8;

const ATTR_OFF_REVISION: u32 = 0x00;
const ATTR_OFF_SDK_VERSION: u32 = 0x04;
const ATTR_OFF_NSPUS: u32 = 0x08;
const ATTR_OFF_SPU_PRIORITY: u32 = 0x0C;
const ATTR_OFF_PPU_PRIORITY: u32 = 0x10;
const ATTR_OFF_EXIT_IF_NO_WORK: u32 = 0x14;
const ATTR_OFF_PREFIX: u32 = 0x15;
const ATTR_OFF_PREFIX_SIZE: u32 = 0x24;
const ATTR_OFF_FLAGS: u32 = 0x28;
const ATTR_OFF_CONTAINER: u32 = 0x2C;
const ATTR_OFF_SWL_PRIORITY: u32 = 0x38;
const ATTR_OFF_SWL_MAX_SPU: u32 = 0x40;
const ATTR_OFF_SWL_IS_PREEM: u32 = 0x44;

// CellSpursWorkloadAttribute layout (size 512, alignas 8).

const CELL_SPURS_WORKLOAD_ATTRIBUTE_ALIGN: u32 = 8;

const WAATTR_OFF_REVISION: u32 = 0x00;
const WAATTR_OFF_PM: u32 = 0x08;
const WAATTR_OFF_SIZE: u32 = 0x0C;
const WAATTR_OFF_DATA: u32 = 0x10;
const WAATTR_OFF_PRIORITY: u32 = 0x18;
const WAATTR_OFF_MIN_CONTENTION: u32 = 0x20;
const WAATTR_OFF_MAX_CONTENTION: u32 = 0x24;

// CellSpurs layout (size 0x1000 SPURS1 / 0x2000 SPURS2, alignas 128).
// Only the named offsets are written; init zeroes the whole block
// first, so unnamed bytes stay at the post-zero default.

const CELL_SPURS_ALIGN: u32 = 128;
const CELL_SPURS_SIZE1: u32 = 4096;
const CELL_SPURS_SIZE2: u32 = 8192;
const CELL_SPURS_NAME_MAX_LENGTH: u32 = 15;
const CELL_SPURS_MAX_WORKLOAD: u32 = 16;
const CELL_SPURS_MAX_WORKLOAD2: u32 = 32;
const CELL_SPURS_MAX_SPU: u32 = 8;
const CELL_SPURS_MAX_PRIORITY: u32 = 16;

const SPURS_OFF_WKL_READY_COUNT_1: u32 = 0x00; // wklReadyCount1[16] (atomic u8)
const SPURS_OFF_WKL_IDLE_SPU_COUNT_OR_RC2: u32 = 0x10; // wklIdleSpuCountOrReadyCount2[16] (atomic u8)
const SPURS_OFF_WKL_MIN_CONTENTION: u32 = 0x40; // wklMinContention[16]
const SPURS_OFF_WKL_MAX_CONTENTION: u32 = 0x50; // wklMaxContention[16] (atomic u8)
const SPURS_OFF_FLAGS1: u32 = 0x74;
const SPURS_OFF_NSPUS: u32 = 0x76;
const SPURS_OFF_WKL_STATE_1: u32 = 0x80; // wklState1[16] (atomic u8)
const SPURS_OFF_WKL_STATUS_1: u32 = 0x90; // wklStatus1[16]
const SPURS_OFF_WKL_EVENT_1: u32 = 0xA0; // wklEvent1[16] (atomic u8)
const SPURS_OFF_WKL_ENABLED: u32 = 0xB0;
const SPURS_OFF_WKL_MSK_B: u32 = 0xB4; // atomic_be u32, system service available-module mask
const SPURS_OFF_SYS_SRV_MSG: u32 = 0xBC; // sysSrvExitBarrier..
const SPURS_OFF_SYS_SRV_MSG_UPDATE_WORKLOAD: u32 = 0xBD;
const SPURS_OFF_SYS_SRV_PREEMPT_WKL_ID: u32 = 0xC0;
const SPURS_OFF_WKL_STATE_2: u32 = 0xD0; // wklState2[16]
const SPURS_OFF_WKL_STATUS_2: u32 = 0xE0; // wklStatus2[16]
const SPURS_OFF_WKL_EVENT_2: u32 = 0xF0; // wklEvent2[16]
const SPURS_OFF_WKL_INFO_1: u32 = 0xB00; // wklInfo1[16] (32 bytes each)
const SPURS_OFF_WKL_INFO_SYS_SRV: u32 = 0xD00;
const SPURS_OFF_WKL_INFO_2: u32 = 0x1000; // wklInfo2[16]

// WorkloadInfo (32 bytes per entry): addr u64 +0x00, arg u64 +0x08,
// size u32 +0x10, uniqueId u8 +0x14, priority[8] +0x18.
const WI_SIZE: u32 = 32;
const WI_OFF_ADDR: u32 = 0x00;
const WI_OFF_ARG: u32 = 0x08;
const WI_OFF_SIZE: u32 = 0x10;
const WI_OFF_UNIQUE_ID: u32 = 0x14;
const WI_OFF_PRIORITY: u32 = 0x18;

// SPURS workload state (wklState1[]/wklState2[]). State 0
// (NON_EXISTENT) is the post-zero default and never explicitly named.
const SPURS_WKL_STATE_PREPARING: u8 = 1;
const SPURS_WKL_STATE_RUNNABLE: u8 = 2;
const SPURS_WKL_STATE_SHUTTING_DOWN: u8 = 3;
const SPURS_WKL_STATE_REMOVABLE: u8 = 4;
const SPURS_OFF_TRACE_BUFFER: u32 = 0x900; // vm::bptr<CellSpursTraceInfo, u64> -- 8-byte BE pointer
const SPURS_OFF_TRACE_DATA_SIZE: u32 = 0x948;
const SPURS_OFF_TRACE_MODE: u32 = 0x950;
const SPURS_OFF_PPU0: u32 = 0xD20;
const SPURS_OFF_PPU1: u32 = 0xD28;
const SPURS_OFF_SPU_TG: u32 = 0xD30;
const SPURS_OFF_SPUS: u32 = 0xD34; // 8 * be_t<u32>
                                   // CellSpurs::eventQueue (0xD5C) and ::eventPort (0xD60) are populated
                                   // by the event-helper-thread spawn path; the bound queue from
                                   // AttachLv2EventQueue lands in the EventPortMux substruct at 0xF00.
const SPURS_OFF_ENABLE_EH: u32 = 0xD68; // atomic_be_t<u32>
const SPURS_OFF_EXCEPTION: u32 = 0xD6C; // be_t<u32>, set on SPURS exception
const SPURS_OFF_FLAGS: u32 = 0xD80;
const SPURS_OFF_SPU_PRIORITY: u32 = 0xD84;
const SPURS_OFF_PPU_PRIORITY: u32 = 0xD88;
const SPURS_OFF_PREFIX: u32 = 0xD8C;
const SPURS_OFF_PREFIX_SIZE: u32 = 0xD9B;
const SPURS_OFF_REVISION: u32 = 0xDA0;
const SPURS_OFF_SDK_VERSION: u32 = 0xDA4;
const SPURS_OFF_SPU_PORT_BITS: u32 = 0xDA8; // atomic_be_t<u64>
const SPURS_OFF_EVENT_PORT_MUX: u32 = 0xF00; // EventPortMux substruct (128 bytes)
const SPURS_OFF_GLOBAL_EXCEPTION_HANDLER: u32 = 0xF80; // atomic_be_t<u64>
const SPURS_OFF_GLOBAL_EXCEPTION_HANDLER_ARGS: u32 = 0xF88; // be_t<u64>

// EventPortMux sub-block offsets (relative to SPURS_OFF_EVENT_PORT_MUX).
const EPM_OFF_SPU_PORT: u32 = 0x04; // be_t<u32>
const EPM_OFF_EVENT_PORT: u32 = 0x10; // be_t<u64>

// CellSpursInfo layout (size 280 bytes per `CHECK_SIZE(CellSpursInfo, 280)`).
const CELL_SPURS_INFO_SIZE: u32 = 280;
const INFO_OFF_NSPUS: u32 = 0x00;
const INFO_OFF_SPU_THREAD_GROUP_PRIORITY: u32 = 0x04;
const INFO_OFF_PPU_THREAD_PRIORITY: u32 = 0x08;
const INFO_OFF_EXIT_IF_NO_WORK: u32 = 0x0C;
const INFO_OFF_SPURS2: u32 = 0x0D;
const INFO_OFF_TRACE_BUFFER: u32 = 0x10; // vm::bptr<void> 4-byte BE pointer
const INFO_OFF_TRACE_BUFFER_SIZE: u32 = 0x18;
const INFO_OFF_TRACE_MODE: u32 = 0x20;
const INFO_OFF_SPU_THREAD_GROUP: u32 = 0x24;
const INFO_OFF_SPU_THREADS: u32 = 0x28; // 8 * be_t<u32>
const INFO_OFF_SPURS_HANDLER_THREAD_0: u32 = 0x48;
const INFO_OFF_SPURS_HANDLER_THREAD_1: u32 = 0x50;
const INFO_OFF_NAME_PREFIX: u32 = 0x58; // 16 bytes
const INFO_OFF_NAME_PREFIX_LENGTH: u32 = 0x68;
const INFO_OFF_DEADLINE_MISS_COUNTER: u32 = 0x6C;
const INFO_OFF_DEADLINE_MEET_COUNTER: u32 = 0x70;

// SAF_* attribute flag bits.

const SAF_NONE: u32 = 0x0;
const SAF_EXIT_IF_NO_WORK: u32 = 0x1;
const SAF_SECOND_VERSION: u32 = 0x4;

// `wklInfoSysSrv.addr` sentinel value set by initialize.
const SPURS_IMG_ADDR_SYS_SRV_WORKLOAD: u32 = 0x100;
const SYS_SRV_WORKLOAD_SIZE: u32 = 0x2200;

/// Dispatch entry point; returns `None` if the NID is not owned here.
pub(crate) fn dispatch(
    runtime: &mut Runtime,
    source: UnitId,
    nid: u32,
    args: &[u64; 9],
) -> Option<()> {
    match nid {
        NID_CELL_SPURS_ATTRIBUTE_INITIALIZE => {
            attribute_initialize(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_INITIALIZE => {
            initialize_bare(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_INITIALIZE_WITH_ATTRIBUTE => {
            initialize_with_attribute(&mut adapter(runtime, source, nid), args, false);
        }
        NID_CELL_SPURS_INITIALIZE_WITH_ATTRIBUTE2 => {
            initialize_with_attribute(&mut adapter(runtime, source, nid), args, true);
        }
        NID_CELL_SPURS_FINALIZE => {
            finalize(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_WORKLOAD_ATTRIBUTE_INITIALIZE => {
            workload_attribute_initialize(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_ADD_WORKLOAD => {
            add_workload(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_ADD_WORKLOAD_WITH_ATTRIBUTE => {
            add_workload_with_attribute(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_SHUTDOWN_WORKLOAD => {
            shutdown_workload(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_WAIT_FOR_WORKLOAD_SHUTDOWN => {
            wait_for_workload_shutdown(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_READY_COUNT_STORE => {
            ready_count_store(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_READY_COUNT_ADD => {
            ready_count_add(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_READY_COUNT_SWAP => {
            ready_count_swap(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_READY_COUNT_COMPARE_AND_SWAP => {
            ready_count_compare_and_swap(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_REQUEST_IDLE_SPU => {
            request_idle_spu(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_SET_MAX_CONTENTION => {
            set_max_contention(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_SET_PRIORITIES => {
            set_priorities(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_SET_PRIORITY => {
            set_priority(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_GET_INFO => {
            get_info(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_ATTACH_LV2_EVENT_QUEUE => {
            attach_lv2_event_queue(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_DETACH_LV2_EVENT_QUEUE => {
            detach_lv2_event_queue(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_SET_EXCEPTION_EVENT_HANDLER => {
            set_exception_event_handler(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_UNSET_EXCEPTION_EVENT_HANDLER => {
            unset_exception_event_handler(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_SET_GLOBAL_EXCEPTION_EVENT_HANDLER => {
            set_global_exception_event_handler(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_UNSET_GLOBAL_EXCEPTION_EVENT_HANDLER => {
            unset_global_exception_event_handler(&mut adapter(runtime, source, nid), args);
        }
        NID_CELL_SPURS_ENABLE_EXCEPTION_EVENT_HANDLER => {
            enable_exception_event_handler(&mut adapter(runtime, source, nid), args);
        }
        _ => return None,
    }
    Some(())
}

fn adapter(runtime: &mut Runtime, source: UnitId, nid: u32) -> RuntimeHleAdapter<'_> {
    RuntimeHleAdapter {
        memory: &mut runtime.memory,
        registry: &mut runtime.registry,
        heap_base: runtime.hle.heap_base,
        heap_ptr: &mut runtime.hle.heap_ptr,
        heap_watermark: &mut runtime.hle.heap_watermark,
        heap_warning_mask: &mut runtime.hle.heap_warning_mask,
        next_id: &mut runtime.hle.next_id,
        source,
        nid,
        mutated: false,
        handlers_without_mutation: &mut runtime.hle.handlers_without_mutation,
    }
}

/// Copy up to `len` prefix bytes into a zero-padded 15-byte buffer.
/// Returns Err if the source range is unmapped; `len == 0` succeeds
/// with the all-zero buffer without touching guest memory.
fn try_read_prefix(
    ctx: &dyn HleContext,
    prefix_addr: u32,
    len: usize,
) -> Result<[u8; 15], HleReadError> {
    let mut buf = [0u8; 15];
    let len = len.min(CELL_SPURS_NAME_MAX_LENGTH as usize);
    if len == 0 {
        return Ok(buf);
    }
    let bytes = ctx.read_guest(prefix_addr as u64, len)?;
    buf[..len].copy_from_slice(bytes);
    Ok(buf)
}

/// `_cellSpursAttributeInitialize(attr, revision, sdkVersion, nSpus,
/// spuPriority, ppuPriority, exitIfNoWork)` -- zero-init the 512-byte
/// attribute block then write the named fields.
///
/// Accepts any `revision` value; the bounds check is on the
/// `cellSpursInitializeWithAttribute*` consumer side.
fn attribute_initialize(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let attr_ptr = args[1] as u32;
    let revision = args[2] as u32;
    let sdk_version = args[3] as u32;
    let n_spus = args[4] as u32;
    let spu_priority = args[5] as i32;
    let ppu_priority = args[6] as i32;
    let exit_if_no_work = args[7] as u8;

    if attr_ptr == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !attr_ptr.is_multiple_of(CELL_SPURS_ATTRIBUTE_ALIGN) {
        ctx.set_return(CELL_SPURS_CORE_ERROR_ALIGN as u64);
        return;
    }

    let zero = vec![0u8; CELL_SPURS_ATTRIBUTE_SIZE as usize];
    if ctx.write_guest(attr_ptr as u64, &zero).is_err() {
        ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64);
        return;
    }

    write_be_u32(ctx, attr_ptr + ATTR_OFF_REVISION, revision);
    write_be_u32(ctx, attr_ptr + ATTR_OFF_SDK_VERSION, sdk_version);
    write_be_u32(ctx, attr_ptr + ATTR_OFF_NSPUS, n_spus);
    write_be_i32(ctx, attr_ptr + ATTR_OFF_SPU_PRIORITY, spu_priority);
    write_be_i32(ctx, attr_ptr + ATTR_OFF_PPU_PRIORITY, ppu_priority);
    write_byte(ctx, attr_ptr + ATTR_OFF_EXIT_IF_NO_WORK, exit_if_no_work);

    ctx.set_return(0);
}

/// `cellSpursInitialize(spurs, nSpus, spuPriority, ppuPriority,
/// exitIfNoWork)` -- bare-args initializer routing through the same
/// internal path as the attribute form (revision = sdkVersion = 0,
/// empty prefix).
fn initialize_bare(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let n_spus = args[2] as i32;
    let spu_priority = args[3] as i32;
    let ppu_priority = args[4] as i32;
    let exit_if_no_work = args[5] as u8;

    let flags = if exit_if_no_work != 0 {
        SAF_EXIT_IF_NO_WORK
    } else {
        SAF_NONE
    };

    let result = spurs_initialize_internal(
        ctx,
        spurs,
        /* revision = */ 0,
        /* sdk_version = */ 0,
        n_spus,
        spu_priority,
        ppu_priority,
        flags,
        /* prefix = */ [0u8; 15],
        /* prefix_size = */ 0,
    );
    ctx.set_return(result as u64);
}

/// `cellSpursInitializeWithAttribute[2](spurs, attr)` -- `is_v2` ORs
/// `SAF_SECOND_VERSION` into the resolved flags.
fn initialize_with_attribute(ctx: &mut dyn HleContext, args: &[u64; 9], is_v2: bool) {
    let spurs = args[1] as u32;
    let attr = args[2] as u32;

    if attr == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !attr.is_multiple_of(CELL_SPURS_ATTRIBUTE_ALIGN) {
        ctx.set_return(CELL_SPURS_CORE_ERROR_ALIGN as u64);
        return;
    }

    // Attribute block reads are guest-pointer-class: an unmapped or
    // out-of-region attr surfaces as INVAL rather than silently
    // substituting a zero-init attribute the caller never wrote.
    let revision = match try_read_be_u32(ctx, attr + ATTR_OFF_REVISION) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if revision > 2 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64);
        return;
    }
    let attr_fields = (|| -> Result<_, HleReadError> {
        Ok((
            try_read_be_u32(ctx, attr + ATTR_OFF_SDK_VERSION)?,
            try_read_be_u32(ctx, attr + ATTR_OFF_NSPUS)? as i32,
            try_read_be_i32(ctx, attr + ATTR_OFF_SPU_PRIORITY)?,
            try_read_be_i32(ctx, attr + ATTR_OFF_PPU_PRIORITY)?,
            try_read_byte(ctx, attr + ATTR_OFF_EXIT_IF_NO_WORK)?,
            try_read_be_u32(ctx, attr + ATTR_OFF_FLAGS)?,
            try_read_be_u32(ctx, attr + ATTR_OFF_PREFIX_SIZE)?,
        ))
    })();
    let (sdk_version, n_spus, spu_priority, ppu_priority, exit_if_no_work, flags_attr, prefix_size) =
        match attr_fields {
            Ok(t) => t,
            Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
        };

    let mut flags = flags_attr;
    if exit_if_no_work != 0 {
        flags |= SAF_EXIT_IF_NO_WORK;
    }
    if is_v2 {
        flags |= SAF_SECOND_VERSION;
    }

    let mut prefix = [0u8; 15];
    let copy_len = (prefix_size as usize).min(CELL_SPURS_NAME_MAX_LENGTH as usize);
    let captured = match try_read_prefix(ctx, attr + ATTR_OFF_PREFIX, copy_len) {
        Ok(buf) => buf,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    prefix[..copy_len].copy_from_slice(&captured[..copy_len]);

    let result = spurs_initialize_internal(
        ctx,
        spurs,
        revision,
        sdk_version,
        n_spus,
        spu_priority,
        ppu_priority,
        flags,
        prefix,
        prefix_size,
    );
    ctx.set_return(result as u64);
}

/// Validate the spurs pointer, zero the 4096 / 8192-byte CellSpurs
/// region (selected by `SAF_SECOND_VERSION`), then patch the named
/// fields. SPU thread group, sync primitives, and helper PPU thread
/// spawn are not part of this path.
#[allow(clippy::too_many_arguments)]
fn spurs_initialize_internal(
    ctx: &mut dyn HleContext,
    spurs: u32,
    revision: u32,
    sdk_version: u32,
    n_spus: i32,
    spu_priority: i32,
    ppu_priority: i32,
    flags: u32,
    prefix: [u8; 15],
    prefix_size: u32,
) -> u32 {
    if spurs == 0 {
        return CELL_SPURS_CORE_ERROR_NULL_POINTER;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        return CELL_SPURS_CORE_ERROR_ALIGN;
    }
    if prefix_size > CELL_SPURS_NAME_MAX_LENGTH {
        return CELL_SPURS_CORE_ERROR_INVAL;
    }
    // 6 user SPUs available; 1 disabled and 1 os-reserved
    if !(1..=6).contains(&n_spus) {
        return CELL_SPURS_CORE_ERROR_INVAL;
    }

    let is_second = (flags & SAF_SECOND_VERSION) != 0;
    let size = if is_second {
        CELL_SPURS_SIZE2
    } else {
        CELL_SPURS_SIZE1
    };

    let zero = vec![0u8; size as usize];
    if ctx.write_guest(spurs as u64, &zero).is_err() {
        // Zero-block witness: a failure here is the caller's bad
        // pointer (e.g. spurs lands in a reserved region). Subsequent
        // field writes are invariant-class because this `Ok` proves
        // [spurs, spurs + size) is writable.
        return CELL_SPURS_CORE_ERROR_INVAL;
    }

    write_be_u32(ctx, spurs + SPURS_OFF_REVISION, revision);
    write_be_u32(ctx, spurs + SPURS_OFF_SDK_VERSION, sdk_version);
    // ppu0 / ppu1 = !0u64 sentinel: "handler/event-helper not spawned".
    write_be_u64(ctx, spurs + SPURS_OFF_PPU0, u64::MAX);
    write_be_u64(ctx, spurs + SPURS_OFF_PPU1, u64::MAX);
    write_be_u32(ctx, spurs + SPURS_OFF_FLAGS, flags);

    // flags1 (u8 at 0x74) is distinct from the u32 flags at 0xD80;
    // max_workloads(), add_workload, and wait_for_workload_shutdown
    // all consult this byte. SF1_32_WORKLOADS=0x40, SF1_EXIT=0x80.
    let flags1: u8 = (if is_second { 0x40u8 } else { 0 })
        | (if (flags & SAF_EXIT_IF_NO_WORK) != 0 {
            0x80u8
        } else {
            0
        });
    write_byte(ctx, spurs + SPURS_OFF_FLAGS1, flags1);

    // prefixSize is a u8 at 0xD9B; the be_t<u32> unk5 at 0xD9C must
    // stay zero. The upstream `prefix_size > CELL_SPURS_NAME_MAX_LENGTH`
    // check guarantees the cast lossless.
    debug_assert!(prefix_size <= CELL_SPURS_NAME_MAX_LENGTH);
    let prefix_size_byte = u8::try_from(prefix_size).unwrap_or(CELL_SPURS_NAME_MAX_LENGTH as u8);
    write_byte(ctx, spurs + SPURS_OFF_PREFIX_SIZE, prefix_size_byte);
    write_bytes(ctx, spurs + SPURS_OFF_PREFIX, &prefix);

    if !is_second {
        write_be_u32(ctx, spurs + SPURS_OFF_WKL_ENABLED, 0xffff);
    }

    // sysSrvPreemptWklId[8] = -1 (no SPU is preempting a workload).
    let preempt_init = [0xffu8; 8];
    write_bytes(ctx, spurs + SPURS_OFF_SYS_SRV_PREEMPT_WKL_ID, &preempt_init);

    write_byte(ctx, spurs + SPURS_OFF_NSPUS, n_spus as u8);
    write_be_i32(ctx, spurs + SPURS_OFF_SPU_PRIORITY, spu_priority);
    write_be_u32(ctx, spurs + SPURS_OFF_PPU_PRIORITY, ppu_priority as u32);

    let sys_srv = spurs + SPURS_OFF_WKL_INFO_SYS_SRV;
    write_be_u64(ctx, sys_srv, SPURS_IMG_ADDR_SYS_SRV_WORKLOAD as u64);
    write_be_u64(ctx, sys_srv + 0x08, 0);
    write_be_u32(ctx, sys_srv + 0x10, SYS_SRV_WORKLOAD_SIZE);
    write_byte(ctx, sys_srv + 0x14, 0xff);

    0
}

/// `cellSpursFinalize(spurs)` -- reset the `ppu0` / `ppu1` sentinels
/// and clear `wklEnabled`. Without spawned handler / event-helper
/// threads or a SPU thread group, the join + destroy half is a no-op.
fn finalize(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    if spurs == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_CORE_ERROR_ALIGN as u64);
        return;
    }

    write_be_u64(ctx, spurs + SPURS_OFF_PPU0, u64::MAX);
    write_be_u64(ctx, spurs + SPURS_OFF_PPU1, u64::MAX);
    write_be_u32(ctx, spurs + SPURS_OFF_WKL_ENABLED, 0);

    ctx.set_return(0);
}

/// `_cellSpursWorkloadAttributeInitialize(attr, revision, sdkVersion,
/// pm, size, data, priority, minCnt, maxCnt)` -- 9-arg SDK wrapper.
///
/// The 9th arg `maxCnt` spills to the caller's parameter save area at
/// `r1 + 48` per PPE 64-bit ABI; `args: [u64; 9]` only covers the
/// 8 register-passed slots. Fails loud (debug panic + release INVAL)
/// rather than silently writing a wrong `maxContention`.
fn workload_attribute_initialize(ctx: &mut dyn HleContext, _args: &[u64; 9]) {
    debug_assert!(
        false,
        "_cellSpursWorkloadAttributeInitialize: maxCnt (9th arg) \
         spills to r1+48; HleContext does not expose the spill yet"
    );
    ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64);
}

/// `cellSpursAddWorkload(spurs, wid_out, pm, size, data, priority,
/// minCnt, maxCnt)` -- allocate the first free workload id (MSB-first
/// scan over `wklEnabled`), populate `wklInfoX[wid]`, and mark the
/// slot enabled. Returns `_AGAIN` when full.
fn add_workload(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid_ptr = args[2] as u32;
    let pm = args[3] as u32;
    let size = args[4] as u32;
    let data = args[5];
    let priority_ptr = args[6] as u32;
    let min_cnt = args[7] as u32;
    let max_cnt = args[8] as u32;

    let result = add_workload_internal(
        ctx,
        spurs,
        wid_ptr,
        pm,
        size,
        data,
        priority_ptr,
        min_cnt,
        max_cnt,
    );
    ctx.set_return(result as u64);
}

/// `cellSpursAddWorkloadWithAttribute(spurs, wid_out, attr)` -- reads
/// pm / size / data / priority / contentions from the attribute block
/// and feeds the same internal path. Only revision 1 is accepted.
fn add_workload_with_attribute(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid_ptr = args[2] as u32;
    let attr = args[3] as u32;

    if attr == 0 {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !attr.is_multiple_of(CELL_SPURS_WORKLOAD_ATTRIBUTE_ALIGN) {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_ALIGN as u64);
        return;
    }
    let revision = match try_read_be_u32(ctx, attr + WAATTR_OFF_REVISION) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    if revision != 1 {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64);
        return;
    }

    let attr_fields = (|| -> Result<_, HleReadError> {
        Ok((
            try_read_be_u32(ctx, attr + WAATTR_OFF_PM)?,
            try_read_be_u32(ctx, attr + WAATTR_OFF_SIZE)?,
            try_read_be_u64(ctx, attr + WAATTR_OFF_DATA)?,
            try_read_be_u32(ctx, attr + WAATTR_OFF_MIN_CONTENTION)?,
            try_read_be_u32(ctx, attr + WAATTR_OFF_MAX_CONTENTION)?,
        ))
    })();
    let (pm, size, data, min_cnt, max_cnt) = match attr_fields {
        Ok(t) => t,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    let priority_addr = attr + WAATTR_OFF_PRIORITY;

    let result = add_workload_internal(
        ctx,
        spurs,
        wid_ptr,
        pm,
        size,
        data,
        priority_addr,
        min_cnt,
        max_cnt,
    );
    ctx.set_return(result as u64);
}

/// Shared body for both AddWorkload variants.
#[allow(clippy::too_many_arguments)]
fn add_workload_internal(
    ctx: &mut dyn HleContext,
    spurs: u32,
    wid_ptr: u32,
    pm: u32,
    size: u32,
    data: u64,
    priority_ptr: u32,
    min_cnt: u32,
    max_cnt: u32,
) -> u32 {
    if spurs == 0 || wid_ptr == 0 || pm == 0 || priority_ptr == 0 {
        return CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        return CELL_SPURS_POLICY_MODULE_ERROR_ALIGN;
    }
    if !pm.is_multiple_of(16) {
        return CELL_SPURS_POLICY_MODULE_ERROR_ALIGN;
    }
    if min_cnt == 0 {
        return CELL_SPURS_POLICY_MODULE_ERROR_INVAL;
    }
    let priority = match try_read_priority_table(ctx, priority_ptr) {
        Ok(t) if t.iter().all(|&b| b <= 15) => t,
        Ok(_) => return CELL_SPURS_POLICY_MODULE_ERROR_INVAL,
        Err(_) => return CELL_SPURS_POLICY_MODULE_ERROR_FAULT,
    };

    // Spurs-block reads from a guest-controlled pointer. An unmapped
    // spurs surfaces as FAULT; a faulted-but-mapped block returns STAT.
    let exception = match try_read_be_u32(ctx, spurs + SPURS_OFF_EXCEPTION) {
        Ok(v) => v,
        Err(_) => return CELL_SPURS_POLICY_MODULE_ERROR_FAULT,
    };
    if exception != 0 {
        return CELL_SPURS_POLICY_MODULE_ERROR_STAT;
    }

    // Bit 31 of wklEnabled = wid 0; scan high-bit-first.
    let (enabled, flags1) = match (
        try_read_be_u32(ctx, spurs + SPURS_OFF_WKL_ENABLED),
        try_read_byte(ctx, spurs + SPURS_OFF_FLAGS1),
    ) {
        (Ok(e), Ok(f)) => (e, f),
        _ => return CELL_SPURS_POLICY_MODULE_ERROR_FAULT,
    };
    let is_second = (flags1 & 0x40) != 0; // SF1_32_WORKLOADS
    let wmax = if is_second {
        CELL_SPURS_MAX_WORKLOAD2
    } else {
        CELL_SPURS_MAX_WORKLOAD
    };

    let wid = (!enabled).leading_zeros();
    if wid >= wmax {
        // *wid stays unmodified on AGAIN -- writing wid=16/32 then
        // returning would describe a slot that does not exist.
        return CELL_SPURS_POLICY_MODULE_ERROR_AGAIN;
    }

    // Guest-controlled out-pointer: bad pointer becomes _FAULT,
    // never a silent drop. wklEnabled is unchanged on the failure
    // branch.
    if try_write_be_u32(ctx, wid_ptr, wid).is_err() {
        return CELL_SPURS_POLICY_MODULE_ERROR_FAULT;
    }

    // uniqueId dedupe: reuse an existing workload's uniqueId when its
    // policy-module address matches the new pm. Bit 31 of `enabled`
    // is wid 0; scan in slot order. RPCS3's `_spurs::add_workload`
    // does the same scan over wklInfo1[i].addr.
    let unique_id = match find_existing_unique_id(ctx, spurs, enabled, pm, is_second) {
        Ok(uid) => uid,
        Err(_) => return CELL_SPURS_POLICY_MODULE_ERROR_FAULT,
    }
    .unwrap_or(wid as u8);

    // Stage per-wid bookkeeping (info, state, status, event,
    // contention) before flipping wklEnabled / wklMskB. A panic in
    // staging leaves the slot reading as unallocated.
    debug_assert!(
        wid < CELL_SPURS_MAX_WORKLOAD || is_second,
        "wid={wid} >= 16 in SPURS1; wkl_info_addr would index past the 4 KiB block"
    );
    let index = wid & 0xf;
    let info_addr = wkl_info_addr(spurs, wid);
    write_be_u64(ctx, info_addr + WI_OFF_ADDR, pm as u64);
    write_be_u64(ctx, info_addr + WI_OFF_ARG, data);
    write_be_u32(ctx, info_addr + WI_OFF_SIZE, size);
    write_bytes(ctx, info_addr + WI_OFF_PRIORITY, &priority);

    let state_arr_off = if wid < 16 {
        SPURS_OFF_WKL_STATE_1
    } else {
        SPURS_OFF_WKL_STATE_2
    };
    write_byte(ctx, spurs + state_arr_off + index, SPURS_WKL_STATE_RUNNABLE);

    let status_arr_off = if wid < 16 {
        SPURS_OFF_WKL_STATUS_1
    } else {
        SPURS_OFF_WKL_STATUS_2
    };
    write_byte(ctx, spurs + status_arr_off + index, 0);
    let event_arr_off = if wid < 16 {
        SPURS_OFF_WKL_EVENT_1
    } else {
        SPURS_OFF_WKL_EVENT_2
    };
    write_byte(ctx, spurs + event_arr_off + index, 0);

    // wklIdleSpuCountOrReadyCount2[wid & 0xf]: SPURS1 idle-SPU count;
    // SPURS2 ready count for wids 16..31. Zero on add either way.
    write_byte(ctx, spurs + SPURS_OFF_WKL_IDLE_SPU_COUNT_OR_RC2 + index, 0);

    if wid < 16 {
        write_byte(ctx, spurs + SPURS_OFF_WKL_READY_COUNT_1 + index, 0);
        // wklMinContention is per-wid for SPURS1 only.
        let min_clamped = if min_cnt > 8 { 8 } else { min_cnt as u8 };
        write_byte(
            ctx,
            spurs + SPURS_OFF_WKL_MIN_CONTENTION + index,
            min_clamped,
        );
    }

    // wklMaxContention[index]: low nibble for wid<16, high nibble
    // for wid>=16; capped at MAX_SPU=8.
    let max_clamped: u8 = if max_cnt > 8 { 8 } else { max_cnt as u8 };
    let mc_addr = spurs + SPURS_OFF_WKL_MAX_CONTENTION + index;
    let prev_mc = match try_read_byte(ctx, mc_addr) {
        Ok(v) => v,
        Err(_) => return CELL_SPURS_POLICY_MODULE_ERROR_FAULT,
    };
    let new_mc = if wid < 16 {
        (prev_mc & 0xf0) | (max_clamped & 0x0f)
    } else {
        (prev_mc & 0x0f) | ((max_clamped & 0x0f) << 4)
    };
    write_byte(ctx, mc_addr, new_mc);

    debug_assert_ne!(wid, 0xFF, "0xFF is the system-service-workload sentinel");
    write_byte(ctx, info_addr + WI_OFF_UNIQUE_ID, unique_id);

    // Commit: flip wklEnabled, then wklMskB, then wake the system
    // service. RPCS3 `_spurs::add_workload` sets the matching bit in
    // wklMskB on alloc (cellSpurs.cpp ~line 2511).
    let new_enabled = enabled | (0x8000_0000u32 >> wid);
    write_be_u32(ctx, spurs + SPURS_OFF_WKL_ENABLED, new_enabled);

    let mask_b = match try_read_be_u32(ctx, spurs + SPURS_OFF_WKL_MSK_B) {
        Ok(v) => v,
        Err(_) => return CELL_SPURS_POLICY_MODULE_ERROR_FAULT,
    };
    write_be_u32(
        ctx,
        spurs + SPURS_OFF_WKL_MSK_B,
        mask_b | (0x8000_0000u32 >> wid),
    );

    write_byte(ctx, spurs + SPURS_OFF_SYS_SRV_MSG_UPDATE_WORKLOAD, 0xff);
    write_byte(ctx, spurs + SPURS_OFF_SYS_SRV_MSG, 0xff);

    0
}

/// Walk every enabled workload (other than the one being inserted)
/// and reuse its uniqueId when its `wklInfo[i].addr` matches `pm`.
/// Returns `Ok(None)` when no match is found and the caller should
/// assign a fresh uniqueId.
fn find_existing_unique_id(
    ctx: &dyn HleContext,
    spurs: u32,
    enabled: u32,
    pm: u32,
    is_second: bool,
) -> Result<Option<u8>, HleReadError> {
    let wmax = if is_second {
        CELL_SPURS_MAX_WORKLOAD2
    } else {
        CELL_SPURS_MAX_WORKLOAD
    };
    for i in 0..wmax {
        if (enabled & (0x8000_0000u32 >> i)) == 0 {
            continue;
        }
        let info = wkl_info_addr(spurs, i);
        // wklInfo[i].addr is the low 32 bits of a be_t<u64> at +0x00;
        // pm pointers fit in 32 bits, so compare on the low word.
        let addr_lo = try_read_be_u32(ctx, info + WI_OFF_ADDR + 4)?;
        if addr_lo == pm {
            return Ok(Some(try_read_byte(ctx, info + WI_OFF_UNIQUE_ID)?));
        }
    }
    Ok(None)
}

/// `cellSpursShutdownWorkload(spurs, wid)` -- transition the
/// workload's state to SHUTTING_DOWN. The SPU-side completion event
/// is out of scope; the state transition is the observable effect.
fn shutdown_workload(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid = args[2] as u32;

    if spurs == 0 {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_ALIGN as u64);
        return;
    }

    let wmax = match try_read_wmax(ctx, spurs) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    if wid >= wmax {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64);
        return;
    }

    let state_arr_off = if wid < 16 {
        SPURS_OFF_WKL_STATE_1
    } else {
        SPURS_OFF_WKL_STATE_2
    };
    let index = wid & 0xf;
    let state_addr = spurs + state_arr_off + index;
    let prev_state = match try_read_byte(ctx, state_addr) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    if prev_state <= SPURS_WKL_STATE_PREPARING {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_STAT as u64);
        return;
    }
    if prev_state == SPURS_WKL_STATE_SHUTTING_DOWN || prev_state == SPURS_WKL_STATE_REMOVABLE {
        // Already shutting down: idempotent CELL_OK.
        ctx.set_return(0);
        return;
    }
    write_byte(ctx, state_addr, SPURS_WKL_STATE_SHUTTING_DOWN);

    let event_arr_off = if wid < 16 {
        SPURS_OFF_WKL_EVENT_1
    } else {
        SPURS_OFF_WKL_EVENT_2
    };
    let event_addr = spurs + event_arr_off + index;
    let prev_event = match try_read_byte(ctx, event_addr) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    write_byte(ctx, event_addr, prev_event | 1);

    write_byte(ctx, spurs + SPURS_OFF_SYS_SRV_MSG, 0xff);
    ctx.set_return(0);
}

/// `cellSpursWaitForWorkloadShutdown(spurs, wid)` -- returns
/// CELL_OK for the no-wait fast path or `_SRCH` if `wid` is not
/// enabled. With no SPU kernel emitting completion events, the
/// nominal wait would never resolve, so this never blocks.
fn wait_for_workload_shutdown(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid = args[2] as u32;

    if spurs == 0 {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_ALIGN as u64);
        return;
    }

    let wmax = match try_read_wmax(ctx, spurs) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    if wid >= wmax {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64);
        return;
    }

    let enabled = match try_read_be_u32(ctx, spurs + SPURS_OFF_WKL_ENABLED) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    if (enabled & (0x8000_0000u32 >> wid)) == 0 {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_SRCH as u64);
        return;
    }

    ctx.set_return(0);
}

/// Error-code bundle picked per call site: `ReadyCount*` returns the
/// POLICY_MODULE namespace; the others return CORE. Same predicates,
/// different numeric codes. `fault` is the code for an unmapped /
/// out-of-region read of the caller's `spurs` pointer; the CORE
/// namespace has no FAULT, so it folds into INVAL there.
struct WorkloadOpErrors {
    null_ptr: u32,
    align: u32,
    inval: u32,
    srch: u32,
    stat: u32,
    fault: u32,
}

const POLICY_MODULE_ERRS: WorkloadOpErrors = WorkloadOpErrors {
    null_ptr: CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER,
    align: CELL_SPURS_POLICY_MODULE_ERROR_ALIGN,
    inval: CELL_SPURS_POLICY_MODULE_ERROR_INVAL,
    srch: CELL_SPURS_POLICY_MODULE_ERROR_SRCH,
    stat: CELL_SPURS_POLICY_MODULE_ERROR_STAT,
    fault: CELL_SPURS_POLICY_MODULE_ERROR_FAULT,
};

const CORE_ERRS: WorkloadOpErrors = WorkloadOpErrors {
    null_ptr: CELL_SPURS_CORE_ERROR_NULL_POINTER,
    align: CELL_SPURS_CORE_ERROR_ALIGN,
    inval: CELL_SPURS_CORE_ERROR_INVAL,
    srch: CELL_SPURS_CORE_ERROR_SRCH,
    stat: CELL_SPURS_CORE_ERROR_STAT,
    fault: CELL_SPURS_CORE_ERROR_INVAL,
};

/// `max_workloads()` per the SF1_32_WORKLOADS bit in `flags1`. Returns
/// `Err` if `flags1` is unmapped / out-of-region.
fn try_read_wmax(ctx: &dyn HleContext, spurs: u32) -> Result<u32, HleReadError> {
    let flags1 = try_read_byte(ctx, spurs + SPURS_OFF_FLAGS1)?;
    Ok(if (flags1 & 0x40) != 0 {
        CELL_SPURS_MAX_WORKLOAD2
    } else {
        CELL_SPURS_MAX_WORKLOAD
    })
}

/// Address of the `readyCount(wid)` byte. SPURS1 wid lives in
/// wklReadyCount1[wid]; SPURS2 wid >= 16 overlaps
/// wklIdleSpuCountOrReadyCount2[wid & 0xf].
fn ready_count_addr(spurs: u32, wid: u32) -> u32 {
    if wid < CELL_SPURS_MAX_WORKLOAD {
        spurs + SPURS_OFF_WKL_READY_COUNT_1 + wid
    } else {
        spurs + SPURS_OFF_WKL_IDLE_SPU_COUNT_OR_RC2 + (wid & 0xf)
    }
}

/// Shared validation prelude: spurs null/align, wid in band, enabled
/// bit set, no pending exception, and optionally `state == RUNNABLE`.
/// Errors come from the caller-supplied namespace bundle.
fn validate_workload_op(
    ctx: &dyn HleContext,
    spurs: u32,
    wid: u32,
    require_runnable: bool,
    errs: &WorkloadOpErrors,
) -> Result<(), u32> {
    if spurs == 0 {
        return Err(errs.null_ptr);
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        return Err(errs.align);
    }
    let wmax = try_read_wmax(ctx, spurs).map_err(|_| errs.fault)?;
    if wid >= wmax {
        return Err(errs.inval);
    }
    let enabled = try_read_be_u32(ctx, spurs + SPURS_OFF_WKL_ENABLED).map_err(|_| errs.fault)?;
    if (enabled & (0x8000_0000u32 >> wid)) == 0 {
        return Err(errs.srch);
    }
    let exception = try_read_be_u32(ctx, spurs + SPURS_OFF_EXCEPTION).map_err(|_| errs.fault)?;
    if exception != 0 {
        return Err(errs.stat);
    }
    if require_runnable {
        let arr = if wid < CELL_SPURS_MAX_WORKLOAD {
            SPURS_OFF_WKL_STATE_1
        } else {
            SPURS_OFF_WKL_STATE_2
        };
        let state = try_read_byte(ctx, spurs + arr + (wid & 0xf)).map_err(|_| errs.fault)?;
        if state != SPURS_WKL_STATE_RUNNABLE {
            return Err(errs.stat);
        }
    }
    Ok(())
}

/// `cellSpursReadyCountStore(spurs, wid, value)` -- store
/// `value & 0xff` into `readyCount(wid)`; `value > 0xff` is INVAL.
fn ready_count_store(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid = args[2] as u32;
    let value = args[3] as u32;

    // Run wid-band validation before the value-overflow check so an
    // OOB wid surfaces _INVAL via the band check, not the value path.
    if let Err(code) = validate_workload_op(ctx, spurs, wid, true, &POLICY_MODULE_ERRS) {
        ctx.set_return(code as u64);
        return;
    }
    if value > 0xff {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64);
        return;
    }

    write_byte(ctx, ready_count_addr(spurs, wid), (value & 0xff) as u8);
    ctx.set_return(0);
}

/// `cellSpursReadyCountAdd(spurs, wid, old_ptr, value)` -- add `value`
/// (s32) saturating-clamped to `[0, 255]`; write prior to `*old_ptr`.
fn ready_count_add(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid = args[2] as u32;
    let old_ptr = args[3] as u32;
    let value = args[4] as i32;

    if old_ptr == 0 {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER as u64);
        return;
    }
    if let Err(code) = validate_workload_op(ctx, spurs, wid, true, &POLICY_MODULE_ERRS) {
        ctx.set_return(code as u64);
        return;
    }

    let addr = ready_count_addr(spurs, wid);
    let prev = match try_read_byte(ctx, addr) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    let next = (prev as i32 + value).clamp(0, 255) as u8;
    write_byte(ctx, addr, next);

    if try_write_be_u32(ctx, old_ptr, prev as u32).is_err() {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64);
        return;
    }
    ctx.set_return(0);
}

/// `cellSpursReadyCountSwap(spurs, wid, old_ptr, swap)` -- replace
/// `readyCount(wid)` with `swap & 0xff`; write prior to `*old_ptr`.
fn ready_count_swap(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid = args[2] as u32;
    let old_ptr = args[3] as u32;
    let swap = args[4] as u32;

    if old_ptr == 0 {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER as u64);
        return;
    }
    if let Err(code) = validate_workload_op(ctx, spurs, wid, true, &POLICY_MODULE_ERRS) {
        ctx.set_return(code as u64);
        return;
    }
    if swap > 0xff {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64);
        return;
    }

    let addr = ready_count_addr(spurs, wid);
    let prev = match try_read_byte(ctx, addr) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    write_byte(ctx, addr, (swap & 0xff) as u8);
    if try_write_be_u32(ctx, old_ptr, prev as u32).is_err() {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64);
        return;
    }
    ctx.set_return(0);
}

/// `cellSpursReadyCountCompareAndSwap(spurs, wid, old_ptr, compare,
/// swap)` -- swap on match; always writes prior to `*old_ptr`.
fn ready_count_compare_and_swap(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid = args[2] as u32;
    let old_ptr = args[3] as u32;
    let compare = args[4] as u32;
    let swap = args[5] as u32;

    if old_ptr == 0 {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER as u64);
        return;
    }
    if let Err(code) = validate_workload_op(ctx, spurs, wid, true, &POLICY_MODULE_ERRS) {
        ctx.set_return(code as u64);
        return;
    }
    if (compare | swap) > 0xff {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64);
        return;
    }

    let addr = ready_count_addr(spurs, wid);
    let prev = match try_read_byte(ctx, addr) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    if prev as u32 == compare {
        write_byte(ctx, addr, (swap & 0xff) as u8);
    }
    if try_write_be_u32(ctx, old_ptr, prev as u32).is_err() {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64);
        return;
    }
    ctx.set_return(0);
}

/// `cellSpursRequestIdleSpu(spurs, wid, count)` -- SPURS1-only: write
/// `count` into the SPURS1 idle-SPU slot at
/// `wklIdleSpuCountOrReadyCount2[wid]`. SPURS2 contexts return STAT.
fn request_idle_spu(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid = args[2] as u32;
    let count = args[3] as u32;

    if spurs == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_CORE_ERROR_ALIGN as u64);
        return;
    }
    // SPURS2 has its own broadcast NIDs; this entrypoint is SPURS1-only.
    let flags1 = match try_read_byte(ctx, spurs + SPURS_OFF_FLAGS1) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if (flags1 & 0x40) != 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_STAT as u64);
        return;
    }
    if wid >= CELL_SPURS_MAX_WORKLOAD || count >= CELL_SPURS_MAX_SPU {
        ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64);
        return;
    }
    let enabled = match try_read_be_u32(ctx, spurs + SPURS_OFF_WKL_ENABLED) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if (enabled & (0x8000_0000u32 >> wid)) == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_SRCH as u64);
        return;
    }
    let exception = match try_read_be_u32(ctx, spurs + SPURS_OFF_EXCEPTION) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if exception != 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_STAT as u64);
        return;
    }

    write_byte(
        ctx,
        spurs + SPURS_OFF_WKL_IDLE_SPU_COUNT_OR_RC2 + wid,
        count as u8,
    );
    ctx.set_return(0);
}

/// `cellSpursSetMaxContention(spurs, wid, maxContention)` -- update
/// the `wklMaxContention[wid % 16]` nibble (low for wid<16, high for
/// wid>=16); value clamps to MAX_SPU=8.
fn set_max_contention(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid = args[2] as u32;
    let max_contention = args[3] as u32;

    if let Err(code) = validate_workload_op(ctx, spurs, wid, false, &CORE_ERRS) {
        ctx.set_return(code as u64);
        return;
    }
    let clamped = if max_contention > CELL_SPURS_MAX_SPU {
        CELL_SPURS_MAX_SPU as u8
    } else {
        max_contention as u8
    };
    let index = wid & 0xf;
    let mc_addr = spurs + SPURS_OFF_WKL_MAX_CONTENTION + index;
    let prev = match try_read_byte(ctx, mc_addr) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    let new_mc = if wid < CELL_SPURS_MAX_WORKLOAD {
        (prev & 0xf0) | (clamped & 0x0f)
    } else {
        (prev & 0x0f) | ((clamped & 0x0f) << 4)
    };
    write_byte(ctx, mc_addr, new_mc);
    ctx.set_return(0);
}

/// `cellSpursSetPriorities(spurs, wid, priorities)` -- copy the
/// 8-byte table at `priorities` into `wklInfoX[wid].priority`. Every
/// byte must be `<= 15`.
fn set_priorities(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid = args[2] as u32;
    let priorities_ptr = args[3] as u32;

    if priorities_ptr == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
        return;
    }
    if let Err(code) = validate_workload_op(ctx, spurs, wid, false, &CORE_ERRS) {
        ctx.set_return(code as u64);
        return;
    }
    let table = match try_read_priority_table(ctx, priorities_ptr) {
        Ok(t) if t.iter().all(|&b| b <= 15) => t,
        Ok(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    let info_addr = wkl_info_addr(spurs, wid);
    write_bytes(ctx, info_addr + WI_OFF_PRIORITY, &table);

    write_byte(ctx, spurs + SPURS_OFF_SYS_SRV_MSG_UPDATE_WORKLOAD, 0xff);
    write_byte(ctx, spurs + SPURS_OFF_SYS_SRV_MSG, 0xff);
    ctx.set_return(0);
}

/// `cellSpursSetPriority(spurs, wid, spuId, priority)` -- write a
/// single byte at `wklInfoX[wid].priority[spuId]`. Requires
/// `priority < 16` and `spuId < spurs->nSpus`.
fn set_priority(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid = args[2] as u32;
    let spu_id = args[3] as u32;
    let priority = args[4] as u32;

    if spurs == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_CORE_ERROR_ALIGN as u64);
        return;
    }
    let wmax = match try_read_wmax(ctx, spurs) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if wid >= wmax {
        ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64);
        return;
    }
    let n_spus = match try_read_byte(ctx, spurs + SPURS_OFF_NSPUS) {
        Ok(v) => v as u32,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if priority >= CELL_SPURS_MAX_PRIORITY || spu_id >= n_spus {
        ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64);
        return;
    }
    let enabled = match try_read_be_u32(ctx, spurs + SPURS_OFF_WKL_ENABLED) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if (enabled & (0x8000_0000u32 >> wid)) == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_SRCH as u64);
        return;
    }
    let exception = match try_read_be_u32(ctx, spurs + SPURS_OFF_EXCEPTION) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if exception != 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_STAT as u64);
        return;
    }

    let info_addr = wkl_info_addr(spurs, wid);
    write_byte(ctx, info_addr + WI_OFF_PRIORITY + spu_id, priority as u8);

    write_byte(ctx, spurs + SPURS_OFF_SYS_SRV_MSG_UPDATE_WORKLOAD, 0xff);
    write_byte(ctx, spurs + SPURS_OFF_SYS_SRV_MSG, 0xff);
    ctx.set_return(0);
}

/// `cellSpursGetInfo(spurs, info)` -- write a 280-byte snapshot of
/// the CellSpurs control-block fields at `info`. SPU-dispatcher
/// outputs (deadline counters, full traceMode tag bits) are zero
/// without a running SPU kernel.
fn get_info(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let info = args[2] as u32;

    if spurs == 0 || info == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_CORE_ERROR_ALIGN as u64);
        return;
    }

    let zero = vec![0u8; CELL_SPURS_INFO_SIZE as usize];
    if ctx.write_guest(info as u64, &zero).is_err() {
        ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64);
        return;
    }

    // Spurs-side fields are guest-pointer-class: an unmapped or
    // out-of-region spurs surfaces as INVAL (the closest CORE-namespace
    // code; CORE has no FAULT) rather than silently returning zeros.
    let spurs_fields = (|| -> Result<_, HleReadError> {
        Ok((
            try_read_be_u32(ctx, spurs + SPURS_OFF_FLAGS)?,
            try_read_byte(ctx, spurs + SPURS_OFF_NSPUS)? as i32,
            try_read_be_i32(ctx, spurs + SPURS_OFF_SPU_PRIORITY)?,
            try_read_be_i32(ctx, spurs + SPURS_OFF_PPU_PRIORITY)?,
            try_read_be_u32(ctx, spurs + SPURS_OFF_SPU_TG)?,
            try_read_be_u64(ctx, spurs + SPURS_OFF_PPU0)?,
            try_read_be_u64(ctx, spurs + SPURS_OFF_PPU1)?,
            try_read_be_u64(ctx, spurs + SPURS_OFF_TRACE_BUFFER)?,
            try_read_be_u64(ctx, spurs + SPURS_OFF_TRACE_DATA_SIZE)?,
            try_read_byte(ctx, spurs + SPURS_OFF_PREFIX_SIZE)?,
        ))
    })();
    let (
        flags,
        n_spus,
        spu_priority,
        ppu_priority,
        spu_tg,
        ppu0,
        ppu1,
        trace_buffer_raw,
        trace_data_size,
        prefix_size,
    ) = match spurs_fields {
        Ok(t) => t,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };

    write_be_i32(ctx, info + INFO_OFF_NSPUS, n_spus);
    write_be_i32(ctx, info + INFO_OFF_SPU_THREAD_GROUP_PRIORITY, spu_priority);
    write_be_i32(ctx, info + INFO_OFF_PPU_THREAD_PRIORITY, ppu_priority);
    write_byte(
        ctx,
        info + INFO_OFF_EXIT_IF_NO_WORK,
        if (flags & SAF_EXIT_IF_NO_WORK) != 0 {
            1
        } else {
            0
        },
    );
    write_byte(
        ctx,
        info + INFO_OFF_SPURS2,
        if (flags & SAF_SECOND_VERSION) != 0 {
            1
        } else {
            0
        },
    );

    // The trace-buffer pointer's low 2 bits encode the trace-mode tag.
    // info->traceBuffer is a 4-byte vm::bptr<void> with the tag bits
    // cleared; info->traceMode receives the tag OR-merged with
    // spurs->traceMode.
    let trace_buffer_addr = (trace_buffer_raw as u32) & !3u32;
    let trace_mode_tag = (trace_buffer_raw as u32) & 3u32;
    let trace_mode_field = match try_read_be_u32(ctx, spurs + SPURS_OFF_TRACE_MODE) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    write_be_u32(ctx, info + INFO_OFF_TRACE_BUFFER, trace_buffer_addr);
    write_be_u64(ctx, info + INFO_OFF_TRACE_BUFFER_SIZE, trace_data_size);
    write_be_u32(
        ctx,
        info + INFO_OFF_TRACE_MODE,
        trace_mode_tag | trace_mode_field,
    );
    write_be_u32(ctx, info + INFO_OFF_SPU_THREAD_GROUP, spu_tg);

    for i in 0..8u32 {
        let spu = match try_read_be_u32(ctx, spurs + SPURS_OFF_SPUS + i * 4) {
            Ok(v) => v,
            Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
        };
        write_be_u32(ctx, info + INFO_OFF_SPU_THREADS + i * 4, spu);
    }

    write_be_u64(ctx, info + INFO_OFF_SPURS_HANDLER_THREAD_0, ppu0);
    write_be_u64(ctx, info + INFO_OFF_SPURS_HANDLER_THREAD_1, ppu1);

    let copy_len = (prefix_size as usize).min(CELL_SPURS_NAME_MAX_LENGTH as usize);
    let prefix = match try_read_prefix(ctx, spurs + SPURS_OFF_PREFIX, copy_len) {
        Ok(buf) => buf,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    // The 16-byte info->namePrefix slot is zero-initialised above, so
    // anything past prefix_size already reads NUL.
    write_bytes(ctx, info + INFO_OFF_NAME_PREFIX, &prefix[..copy_len]);
    write_be_u32(ctx, info + INFO_OFF_NAME_PREFIX_LENGTH, prefix_size as u32);

    ctx.set_return(0);
}

/// `cellSpursAttachLv2EventQueue(spurs, queue, port_ptr, isDynamic)`
/// -- bind a (queue, port) into the `eventPortMux` substruct and
/// flip the matching `spuPortBits` bit. The SPU thread-group
/// connect-event call is a no-op without a running SPU thread group.
fn attach_lv2_event_queue(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let queue = args[2] as u32;
    let port_ptr = args[3] as u32;
    let is_dynamic = args[4] as i32;

    if spurs == 0 || port_ptr == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_CORE_ERROR_ALIGN as u64);
        return;
    }
    let exception = match try_read_be_u32(ctx, spurs + SPURS_OFF_EXCEPTION) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if exception != 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_STAT as u64);
        return;
    }

    // Static: caller's *port already names the desired SPU port.
    // Dynamic: pick the first untaken bit in [0x10, 0x40), since the
    // SPU thread-group allocator that would normally choose isn't
    // running.
    let port = if is_dynamic == 0 {
        let p = match try_read_byte(ctx, port_ptr) {
            Ok(v) => v,
            Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
        };
        if p > 0x3F {
            ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64);
            return;
        }
        p
    } else {
        let bits = match try_read_be_u64(ctx, spurs + SPURS_OFF_SPU_PORT_BITS) {
            Ok(v) => v,
            Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
        };
        let mut chosen: Option<u8> = None;
        for p in 0x10u8..0x40 {
            if (bits & (1u64 << p)) == 0 {
                chosen = Some(p);
                break;
            }
        }
        let Some(p) = chosen else {
            ctx.set_return(CELL_SPURS_CORE_ERROR_BUSY as u64);
            return;
        };
        // Guest-supplied out-pointer: bad address becomes _INVAL.
        if try_write_byte(ctx, port_ptr, p).is_err() {
            ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64);
            return;
        }
        p
    };

    write_be_u32(
        ctx,
        spurs + SPURS_OFF_EVENT_PORT_MUX + EPM_OFF_SPU_PORT,
        port as u32,
    );
    write_be_u64(
        ctx,
        spurs + SPURS_OFF_EVENT_PORT_MUX + EPM_OFF_EVENT_PORT,
        queue as u64,
    );
    let prev_bits = match try_read_be_u64(ctx, spurs + SPURS_OFF_SPU_PORT_BITS) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    write_be_u64(
        ctx,
        spurs + SPURS_OFF_SPU_PORT_BITS,
        prev_bits | (1u64 << port),
    );

    ctx.set_return(0);
}

/// `cellSpursDetachLv2EventQueue(spurs, port)` -- clear the port bit
/// in `spuPortBits` and zero the bound queue slot if the detached
/// port matches. A clear bit returns `_SRCH` (SDK >= 0x180000).
fn detach_lv2_event_queue(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let port = args[2] as u8;

    if spurs == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_CORE_ERROR_ALIGN as u64);
        return;
    }
    let exception = match try_read_be_u32(ctx, spurs + SPURS_OFF_EXCEPTION) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if exception != 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_STAT as u64);
        return;
    }
    if port > 0x3F {
        ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64);
        return;
    }

    let prev_bits = match try_read_be_u64(ctx, spurs + SPURS_OFF_SPU_PORT_BITS) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    let mask = 1u64 << port;
    if (prev_bits & mask) == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_SRCH as u64);
        return;
    }
    write_be_u64(ctx, spurs + SPURS_OFF_SPU_PORT_BITS, prev_bits & !mask);

    // The mux substruct only tracks the last-bound port pair.
    // Detaching a different port clears only its `spuPortBits` bit.
    let bound_port = match try_read_be_u32(ctx, spurs + SPURS_OFF_EVENT_PORT_MUX + EPM_OFF_SPU_PORT)
    {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if bound_port == port as u32 {
        write_be_u32(ctx, spurs + SPURS_OFF_EVENT_PORT_MUX + EPM_OFF_SPU_PORT, 0);
        write_be_u64(
            ctx,
            spurs + SPURS_OFF_EVENT_PORT_MUX + EPM_OFF_EVENT_PORT,
            0,
        );
    }

    ctx.set_return(0);
}

/// `cellSpursSetExceptionEventHandler(spurs, wid, hook, taskset)` --
/// `wid == 0xffffffff` is the global-handler sentinel and routes to
/// the same write as `cellSpursSetGlobalExceptionEventHandler`. The
/// per-workload slot is not laid out in the spec; valid `wid` returns
/// CELL_OK with no field write to match the canonical UNIMPLEMENTED
/// stub.
fn set_exception_event_handler(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid = args[2] as u32;
    let hook = args[3] as u32;
    let taskset = args[4] as u32;

    if spurs == 0 {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_ALIGN as u64);
        return;
    }

    if wid == u32::MAX {
        if hook == 0 {
            ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
            return;
        }
        let exception = match try_read_be_u32(ctx, spurs + SPURS_OFF_EXCEPTION) {
            Ok(v) => v,
            Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
        };
        if exception != 0 {
            ctx.set_return(CELL_SPURS_CORE_ERROR_STAT as u64);
            return;
        }
        let prev = match try_read_be_u64(ctx, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER) {
            Ok(v) => v,
            Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
        };
        if prev != 0 {
            ctx.set_return(CELL_SPURS_CORE_ERROR_BUSY as u64);
            return;
        }
        // On the sentinel path, `taskset` is the handler-args pointer
        // (the third arg of SetGlobalExceptionEventHandler).
        write_be_u64(
            ctx,
            spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER_ARGS,
            taskset as u64,
        );
        write_be_u64(ctx, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER, hook as u64);
        ctx.set_return(0);
        return;
    }

    let wmax = match try_read_wmax(ctx, spurs) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    if wid >= wmax {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64);
        return;
    }
    let enabled = match try_read_be_u32(ctx, spurs + SPURS_OFF_WKL_ENABLED) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    if (enabled & (0x8000_0000u32 >> wid)) == 0 {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_SRCH as u64);
        return;
    }
    // No spec-defined per-workload handler slot; CELL_OK without a
    // field write matches the canonical UNIMPLEMENTED stub.
    ctx.set_return(0);
}

/// `cellSpursUnsetExceptionEventHandler(spurs, wid)` -- mirror of
/// `set_exception_event_handler`: sentinel routes to the global-clear
/// path; valid wid is a CELL_OK no-op.
fn unset_exception_event_handler(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let wid = args[2] as u32;

    if spurs == 0 {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_ALIGN as u64);
        return;
    }

    if wid == u32::MAX {
        let exception = match try_read_be_u32(ctx, spurs + SPURS_OFF_EXCEPTION) {
            Ok(v) => v,
            Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
        };
        if exception != 0 {
            ctx.set_return(CELL_SPURS_CORE_ERROR_STAT as u64);
            return;
        }
        write_be_u64(ctx, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER_ARGS, 0);
        write_be_u64(ctx, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER, 0);
        ctx.set_return(0);
        return;
    }

    let wmax = match try_read_wmax(ctx, spurs) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    if wid >= wmax {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64);
        return;
    }
    let enabled = match try_read_be_u32(ctx, spurs + SPURS_OFF_WKL_ENABLED) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64),
    };
    if (enabled & (0x8000_0000u32 >> wid)) == 0 {
        ctx.set_return(CELL_SPURS_POLICY_MODULE_ERROR_SRCH as u64);
        return;
    }
    ctx.set_return(0);
}

/// `cellSpursSetGlobalExceptionEventHandler(spurs, eaHandler, arg)`
/// -- write `globalSpuExceptionHandlerArgs` then
/// `globalSpuExceptionHandler`. Returns BUSY when a handler is
/// already registered (caller must Unset first).
fn set_global_exception_event_handler(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let ea_handler = args[2] as u32;
    let arg = args[3] as u32;

    if spurs == 0 || ea_handler == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_CORE_ERROR_ALIGN as u64);
        return;
    }
    let exception = match try_read_be_u32(ctx, spurs + SPURS_OFF_EXCEPTION) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if exception != 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_STAT as u64);
        return;
    }
    let prev = match try_read_be_u64(ctx, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if prev != 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_BUSY as u64);
        return;
    }

    write_be_u64(
        ctx,
        spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER_ARGS,
        arg as u64,
    );
    write_be_u64(
        ctx,
        spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER,
        ea_handler as u64,
    );
    ctx.set_return(0);
}

/// `cellSpursUnsetGlobalExceptionEventHandler(spurs)` -- clear both
/// handler slots; rejects with STAT if an exception is pending.
fn unset_global_exception_event_handler(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;

    if spurs == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_CORE_ERROR_ALIGN as u64);
        return;
    }
    let exception = match try_read_be_u32(ctx, spurs + SPURS_OFF_EXCEPTION) {
        Ok(v) => v,
        Err(_) => return ctx.set_return(CELL_SPURS_CORE_ERROR_INVAL as u64),
    };
    if exception != 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_STAT as u64);
        return;
    }

    write_be_u64(ctx, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER_ARGS, 0);
    write_be_u64(ctx, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER, 0);
    ctx.set_return(0);
}

/// `cellSpursEnableExceptionEventHandler(spurs, flag)` -- exchange
/// `enableEH` with `flag ? 1 : 0`. The
/// `sys_spu_thread_group_{connect,disconnect}_event` side effect is
/// not emitted (no SPU thread group exists yet).
fn enable_exception_event_handler(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let spurs = args[1] as u32;
    let flag = args[2] as u8;

    if spurs == 0 {
        ctx.set_return(CELL_SPURS_CORE_ERROR_NULL_POINTER as u64);
        return;
    }
    if !spurs.is_multiple_of(CELL_SPURS_ALIGN) {
        ctx.set_return(CELL_SPURS_CORE_ERROR_ALIGN as u64);
        return;
    }

    let new = if flag != 0 { 1u32 } else { 0u32 };
    write_be_u32(ctx, spurs + SPURS_OFF_ENABLE_EH, new);
    ctx.set_return(0);
}

/// Address of the 32-byte `wklInfo[wid]` block. SPURS1 wids live in
/// `wklInfo1[wid]`; SPURS2 wid >= 16 lives in `wklInfo2[wid & 0xf]`.
fn wkl_info_addr(spurs: u32, wid: u32) -> u32 {
    let base = if wid < CELL_SPURS_MAX_WORKLOAD {
        SPURS_OFF_WKL_INFO_1
    } else {
        SPURS_OFF_WKL_INFO_2
    };
    spurs + base + (wid & 0xf) * WI_SIZE
}

fn try_read_priority_table(ctx: &dyn HleContext, addr: u32) -> Result<[u8; 8], HleReadError> {
    let mut buf = [0u8; 8];
    let bytes = ctx.read_guest(addr as u64, 8)?;
    buf.copy_from_slice(bytes);
    Ok(buf)
}

fn try_read_be_u64(ctx: &dyn HleContext, addr: u32) -> Result<u64, HleReadError> {
    let bytes = ctx.read_guest(addr as u64, 8)?;
    let mut buf = [0u8; 8];
    buf.copy_from_slice(bytes);
    Ok(u64::from_be_bytes(buf))
}

// Two write classes:
//
// - `write_*`: addresses inside a CellSpurs / Attribute /
//   WorkloadAttribute block already proven writable by its zero-init.
//   Failure means an allocator/commit-pipeline bug, so `.expect`.
// - `try_write_*`: guest-controlled pointers (out-pointers caller
//   supplied). Propagate Err so the handler maps it to a faithful
//   error code instead of silently dropping the write.

fn write_be_u32(ctx: &mut dyn HleContext, addr: u32, value: u32) {
    ctx.write_guest(addr as u64, &value.to_be_bytes())
        .expect("cellSpurs: invariant-class field write failed past a zero-init witness");
}

fn write_be_i32(ctx: &mut dyn HleContext, addr: u32, value: i32) {
    ctx.write_guest(addr as u64, &value.to_be_bytes())
        .expect("cellSpurs: invariant-class field write failed past a zero-init witness");
}

fn write_be_u64(ctx: &mut dyn HleContext, addr: u32, value: u64) {
    ctx.write_guest(addr as u64, &value.to_be_bytes())
        .expect("cellSpurs: invariant-class field write failed past a zero-init witness");
}

fn write_byte(ctx: &mut dyn HleContext, addr: u32, value: u8) {
    ctx.write_guest(addr as u64, &[value])
        .expect("cellSpurs: invariant-class field write failed past a zero-init witness");
}

fn write_bytes(ctx: &mut dyn HleContext, addr: u32, bytes: &[u8]) {
    ctx.write_guest(addr as u64, bytes)
        .expect("cellSpurs: invariant-class block write failed past a zero-init witness");
}

fn try_write_be_u32(
    ctx: &mut dyn HleContext,
    addr: u32,
    value: u32,
) -> Result<(), crate::hle::context::HleWriteError> {
    ctx.write_guest(addr as u64, &value.to_be_bytes())
}

fn try_write_byte(
    ctx: &mut dyn HleContext,
    addr: u32,
    value: u8,
) -> Result<(), crate::hle::context::HleWriteError> {
    ctx.write_guest(addr as u64, &[value])
}

fn try_read_be_u32(ctx: &dyn HleContext, addr: u32) -> Result<u32, HleReadError> {
    let bytes = ctx.read_guest(addr as u64, 4)?;
    let mut buf = [0u8; 4];
    buf.copy_from_slice(bytes);
    Ok(u32::from_be_bytes(buf))
}

fn try_read_be_i32(ctx: &dyn HleContext, addr: u32) -> Result<i32, HleReadError> {
    Ok(try_read_be_u32(ctx, addr)? as i32)
}

fn try_read_byte(ctx: &dyn HleContext, addr: u32) -> Result<u8, HleReadError> {
    let bytes = ctx.read_guest(addr as u64, 1)?;
    Ok(bytes[0])
}

// Compile-time tripwires for cross-field offset relationships. A
// rename or renumber that breaks one of these would produce silent
// data corruption (writing the wrong field, indexing past a region
// limit). Asserting at compile time fails the build first.
#[allow(dead_code)]
const _COMPILE_TIME_OFFSETS: () = {
    // CellSpurs region bounds.
    assert!(SPURS_OFF_PPU0 < CELL_SPURS_SIZE1);
    assert!(SPURS_OFF_PPU0 < CELL_SPURS_SIZE2);
    assert!(SPURS_OFF_REVISION < CELL_SPURS_SIZE1);
    assert!(SPURS_OFF_EXCEPTION < CELL_SPURS_SIZE1);
    // exception (be_t<u32>) is immediately followed by sys_spu_image
    // spuImg at 0xD70. Tightening SPURS_OFF_EXCEPTION to enableEH
    // (0xD68) by mistake would still pass the size check; this pins
    // the gap so the misread fails compile.
    const SPU_IMG_START: u32 = 0xD70;
    assert!(SPURS_OFF_EXCEPTION + 4 == SPU_IMG_START);
    assert!(SPU_IMG_START < CELL_SPURS_SIZE1);
    assert!(SPURS_OFF_WKL_MSK_B + 4 == 0xB8);
    assert!(SPURS_OFF_PREFIX + 15 == SPURS_OFF_PREFIX_SIZE);
    // prefixSize is a u8; unk5 (u32) follows at 0xD9C. Renumbering
    // prefixSize into a 4-byte field would clobber unk5.
    assert!(SPURS_OFF_PREFIX_SIZE + 1 == 0xD9C);
    assert!(SPURS_OFF_REVISION + 4 == SPURS_OFF_SDK_VERSION);
    assert!(SPURS_OFF_FLAGS + 4 == SPURS_OFF_SPU_PRIORITY);
    assert!(SPURS_OFF_SPU_PRIORITY + 4 == SPURS_OFF_PPU_PRIORITY);
    // wklInfo2[16] must fit inside the SPURS2 8 KiB region but live
    // past the SPURS1 4 KiB limit (the SPURS2-only bank).
    assert!(SPURS_OFF_WKL_INFO_2 + 16 * WI_SIZE <= CELL_SPURS_SIZE2);
    assert!(SPURS_OFF_WKL_INFO_2 >= CELL_SPURS_SIZE1);
    assert!(ATTR_OFF_PREFIX + 15 == ATTR_OFF_PREFIX_SIZE);
    assert!(ATTR_OFF_REVISION + 4 == ATTR_OFF_SDK_VERSION);
    assert!(ATTR_OFF_NSPUS + 4 == ATTR_OFF_SPU_PRIORITY);
    assert!(ATTR_OFF_SPU_PRIORITY + 4 == ATTR_OFF_PPU_PRIORITY);
    assert!(ATTR_OFF_PPU_PRIORITY + 4 == ATTR_OFF_EXIT_IF_NO_WORK);
    assert!(ATTR_OFF_FLAGS + 4 == ATTR_OFF_CONTAINER);
    assert!(ATTR_OFF_SWL_PRIORITY + 8 == ATTR_OFF_SWL_MAX_SPU);
    assert!(ATTR_OFF_SWL_MAX_SPU + 4 == ATTR_OFF_SWL_IS_PREEM);
    // EventPortMux substruct (128 bytes) and the deepest written
    // field inside it (eventPort at +0x10) must stay in SPURS1 range.
    assert!(SPURS_OFF_EVENT_PORT_MUX + 128 <= CELL_SPURS_SIZE1);
    assert!(SPURS_OFF_EVENT_PORT_MUX + EPM_OFF_EVENT_PORT + 8 <= CELL_SPURS_SIZE1);
    // globalSpuExceptionHandler / args at 0xF80 / 0xF88.
    assert!(SPURS_OFF_GLOBAL_EXCEPTION_HANDLER + 8 == SPURS_OFF_GLOBAL_EXCEPTION_HANDLER_ARGS);
    assert!(SPURS_OFF_GLOBAL_EXCEPTION_HANDLER_ARGS + 8 <= CELL_SPURS_SIZE1);
    // CellSpursInfo: padding[164] starts at 0x74, so total = 280.
    assert!(INFO_OFF_NAME_PREFIX + 16 == INFO_OFF_NAME_PREFIX_LENGTH);
    assert!(INFO_OFF_NAME_PREFIX_LENGTH + 4 == INFO_OFF_DEADLINE_MISS_COUNTER);
    assert!(INFO_OFF_DEADLINE_MISS_COUNTER + 4 == INFO_OFF_DEADLINE_MEET_COUNTER);
    assert!(INFO_OFF_SPU_THREADS + 32 == INFO_OFF_SPURS_HANDLER_THREAD_0);
    assert!(INFO_OFF_SPURS_HANDLER_THREAD_0 + 8 == INFO_OFF_SPURS_HANDLER_THREAD_1);
    assert!(INFO_OFF_DEADLINE_MEET_COUNTER + 4 + 164 == CELL_SPURS_INFO_SIZE);
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::Runtime;
    use cellgov_event::UnitId;
    use cellgov_exec::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    fn fixture() -> (Runtime, UnitId) {
        // 8 MiB region: SPURS test addresses fit in 0x4_xxxx +
        // 0x6_xxxx scratch; heap_base at 1 MiB. Bigger memories blow
        // the test process when ~25 instances run in parallel.
        let mut rt = Runtime::new(GuestMemory::new(0x80_0000), Budget::new(1), 100);
        let unit_id = UnitId::new(0);
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.set_hle_heap_base(0x10_0000);
        (rt, unit_id)
    }

    fn read_u32_be(rt: &Runtime, addr: u32) -> u32 {
        let m = rt.memory().as_bytes();
        u32::from_be_bytes([
            m[addr as usize],
            m[addr as usize + 1],
            m[addr as usize + 2],
            m[addr as usize + 3],
        ])
    }

    fn read_u64_be(rt: &Runtime, addr: u32) -> u64 {
        let m = rt.memory().as_bytes();
        let mut buf = [0u8; 8];
        buf.copy_from_slice(&m[addr as usize..addr as usize + 8]);
        u64::from_be_bytes(buf)
    }

    fn read_byte_at(rt: &Runtime, addr: u32) -> u8 {
        rt.memory().as_bytes()[addr as usize]
    }

    fn drain_return(rt: &mut Runtime, unit: UnitId) -> u64 {
        rt.registry_mut()
            .drain_syscall_return(unit)
            .expect("set_return called")
    }

    #[test]
    fn attribute_initialize_writes_fields() {
        let (mut rt, unit_id) = fixture();
        let attr_ptr: u32 = 0x4_0000;
        let args: [u64; 9] = [
            0x10000,
            attr_ptr as u64,
            /* revision = */ 1,
            /* sdkVersion = */ 0x12345678,
            /* nSpus = */ 5,
            /* spuPriority = */ 100,
            /* ppuPriority = */ 200,
            /* exitIfNoWork = */ 1,
            0,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ATTRIBUTE_INITIALIZE, &args);

        assert_eq!(drain_return(&mut rt, unit_id), 0, "CELL_OK");
        assert_eq!(read_u32_be(&rt, attr_ptr + ATTR_OFF_REVISION), 1);
        assert_eq!(
            read_u32_be(&rt, attr_ptr + ATTR_OFF_SDK_VERSION),
            0x12345678
        );
        assert_eq!(read_u32_be(&rt, attr_ptr + ATTR_OFF_NSPUS), 5);
        assert_eq!(read_u32_be(&rt, attr_ptr + ATTR_OFF_SPU_PRIORITY), 100);
        assert_eq!(read_u32_be(&rt, attr_ptr + ATTR_OFF_PPU_PRIORITY), 200);
        assert_eq!(read_byte_at(&rt, attr_ptr + ATTR_OFF_EXIT_IF_NO_WORK), 1);
    }

    #[test]
    fn attribute_initialize_null_pointer_rejected() {
        let (mut rt, unit_id) = fixture();
        let args: [u64; 9] = [0x10000, 0, 1, 0, 1, 100, 200, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ATTRIBUTE_INITIALIZE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_NULL_POINTER as u64
        );
    }

    #[test]
    fn attribute_initialize_misaligned_rejected() {
        let (mut rt, unit_id) = fixture();
        let args: [u64; 9] = [0x10000, 0x4_0001, 1, 0, 1, 100, 200, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ATTRIBUTE_INITIALIZE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_ALIGN as u64
        );
    }

    #[test]
    fn initialize_bare_populates_spurs_block() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000; // 128-byte aligned
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            /* nSpus = */ 2,
            /* spuPriority = */ 250,
            /* ppuPriority = */ 1000,
            /* exitIfNoWork = */ 1,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_INITIALIZE, &args);

        assert_eq!(drain_return(&mut rt, unit_id), 0, "CELL_OK");
        assert_eq!(read_u32_be(&rt, spurs + SPURS_OFF_REVISION), 0);
        assert_eq!(read_u64_be(&rt, spurs + SPURS_OFF_PPU0), u64::MAX);
        assert_eq!(read_u64_be(&rt, spurs + SPURS_OFF_PPU1), u64::MAX);
        assert_eq!(
            read_u32_be(&rt, spurs + SPURS_OFF_FLAGS),
            SAF_EXIT_IF_NO_WORK
        );
        assert_eq!(read_byte_at(&rt, spurs + SPURS_OFF_NSPUS), 2);
        assert_eq!(read_u32_be(&rt, spurs + SPURS_OFF_SPU_PRIORITY), 250);
        assert_eq!(read_u32_be(&rt, spurs + SPURS_OFF_PPU_PRIORITY), 1000);
        assert_eq!(
            read_u32_be(&rt, spurs + SPURS_OFF_WKL_ENABLED),
            0xffff,
            "SPURS1 wklEnabled"
        );
        for i in 0..8 {
            assert_eq!(
                read_byte_at(&rt, spurs + SPURS_OFF_SYS_SRV_PREEMPT_WKL_ID + i),
                0xff
            );
        }
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_WKL_INFO_SYS_SRV),
            SPURS_IMG_ADDR_SYS_SRV_WORKLOAD as u64
        );
        assert_eq!(
            read_u32_be(&rt, spurs + SPURS_OFF_WKL_INFO_SYS_SRV + 0x10),
            SYS_SRV_WORKLOAD_SIZE
        );
        assert_eq!(
            read_byte_at(&rt, spurs + SPURS_OFF_WKL_INFO_SYS_SRV + 0x14),
            0xff
        );
    }

    #[test]
    fn initialize_bare_misaligned_spurs_rejected() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0040; // 64-byte aligned, NOT 128-byte aligned
        let args: [u64; 9] = [0x10000, spurs as u64, 2, 250, 1000, 1, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_INITIALIZE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_ALIGN as u64
        );
    }

    #[test]
    fn initialize_bare_null_spurs_rejected() {
        let (mut rt, unit_id) = fixture();
        let args: [u64; 9] = [0x10000, 0, 2, 250, 1000, 1, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_INITIALIZE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_NULL_POINTER as u64
        );
    }

    #[test]
    fn initialize_with_attribute_v2_sets_second_version_flag() {
        let (mut rt, unit_id) = fixture();
        let attr_ptr: u32 = 0x2_0000;
        let spurs: u32 = 0x4_0000;

        // First seed an attribute via _cellSpursAttributeInitialize.
        let attr_args: [u64; 9] = [
            0x10000,
            attr_ptr as u64,
            /* revision = */ 2,
            /* sdkVersion = */ 0,
            /* nSpus = */ 1,
            /* spuPriority = */ 128,
            /* ppuPriority = */ 1000,
            /* exitIfNoWork = */ 0,
            0,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ATTRIBUTE_INITIALIZE, &attr_args);
        let _ = drain_return(&mut rt, unit_id);

        let init_args: [u64; 9] = [0x10000, spurs as u64, attr_ptr as u64, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_INITIALIZE_WITH_ATTRIBUTE2,
            &init_args,
        );
        assert_eq!(drain_return(&mut rt, unit_id), 0, "CELL_OK");

        let flags = read_u32_be(&rt, spurs + SPURS_OFF_FLAGS);
        assert_eq!(
            flags & SAF_SECOND_VERSION,
            SAF_SECOND_VERSION,
            "WithAttribute2 sets SAF_SECOND_VERSION"
        );
        // For SPURS2 the wklEnabled SPURS1 default is NOT written.
        assert_eq!(read_u32_be(&rt, spurs + SPURS_OFF_WKL_ENABLED), 0);
        assert_eq!(read_u32_be(&rt, spurs + SPURS_OFF_REVISION), 2);
    }

    #[test]
    fn initialize_with_attribute_rejects_revision_above_two() {
        let (mut rt, unit_id) = fixture();
        let attr_ptr: u32 = 0x2_0000;
        let spurs: u32 = 0x4_0000;

        // Manually plant a CellSpursAttribute with revision = 3.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(attr_ptr as u64), 4).unwrap(),
            &3u32.to_be_bytes(),
        );

        let init_args: [u64; 9] = [0x10000, spurs as u64, attr_ptr as u64, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_INITIALIZE_WITH_ATTRIBUTE,
            &init_args,
        );
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn finalize_clears_handler_thread_sentinels() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        let init_args: [u64; 9] = [0x10000, spurs as u64, 2, 250, 1000, 1, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_INITIALIZE, &init_args);
        let _ = drain_return(&mut rt, unit_id);

        let fin_args: [u64; 9] = [0x10000, spurs as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_FINALIZE, &fin_args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);

        assert_eq!(read_u64_be(&rt, spurs + SPURS_OFF_PPU0), u64::MAX);
        assert_eq!(read_u64_be(&rt, spurs + SPURS_OFF_PPU1), u64::MAX);
        assert_eq!(read_u32_be(&rt, spurs + SPURS_OFF_WKL_ENABLED), 0);
    }

    #[test]
    fn finalize_misaligned_rejected() {
        let (mut rt, unit_id) = fixture();
        let args: [u64; 9] = [0x10000, 0x4_0040, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_FINALIZE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_ALIGN as u64
        );
    }

    #[test]
    fn finalize_null_rejected() {
        let (mut rt, unit_id) = fixture();
        let args: [u64; 9] = [0x10000, 0, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_FINALIZE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_NULL_POINTER as u64
        );
    }

    /// Drive `cellSpursInitialize` against a SPURS1 block at `spurs`.
    fn init_spurs(rt: &mut Runtime, unit_id: UnitId, spurs: u32) {
        let args: [u64; 9] = [0x10000, spurs as u64, 1, 250, 1000, 1, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_INITIALIZE, &args);
        let _ = drain_return(rt, unit_id);
    }

    /// Plant a revision=2 attribute at `attr_ptr` then drive
    /// `cellSpursInitializeWithAttribute2` against `spurs`.
    fn init_spurs_v2(rt: &mut Runtime, unit_id: UnitId, spurs: u32, attr_ptr: u32) {
        let attr_args: [u64; 9] = [0x10000, attr_ptr as u64, 2, 0, 1, 128, 1000, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ATTRIBUTE_INITIALIZE, &attr_args);
        let _ = drain_return(rt, unit_id);
        let init_args: [u64; 9] = [0x10000, spurs as u64, attr_ptr as u64, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_INITIALIZE_WITH_ATTRIBUTE2,
            &init_args,
        );
        let _ = drain_return(rt, unit_id);
    }

    /// Plant an all-zero (valid) 8-byte priority table at `addr`.
    fn plant_priority_zero(rt: &mut Runtime, addr: u32) {
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(addr as u64), 8).unwrap(),
            &[0u8; 8],
        );
    }

    #[test]
    fn add_workload_allocates_first_slot_after_init() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);

        let wid_ptr: u32 = 0x6_0000;
        let pm: u32 = 0x6_1000; // 16-byte aligned
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);

        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            wid_ptr as u64,
            pm as u64,
            /* size = */ 0x1000,
            /* data = */ 0xdead_beef,
            priority_ptr as u64,
            /* minCnt = */ 1,
            /* maxCnt = */ 2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0, "CELL_OK");

        // wklEnabled init = 0x0000_ffff (low 16 bits reserved); the
        // first MSB-first free slot is wid 0, so AddWorkload assigns
        // 0 and sets bit 0x80000000.
        assert_eq!(read_u32_be(&rt, wid_ptr), 0);
        assert_eq!(read_u32_be(&rt, spurs + SPURS_OFF_WKL_ENABLED), 0x8000_ffff);
    }

    #[test]
    fn add_workload_returns_again_when_no_high_bits_free() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);

        // Plant wklEnabled = 0xffff_ffff (all 32 bits set ->
        // countl_one = 32 >= wmax=16 -> AGAIN).
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_ENABLED) as u64),
                4,
            )
            .unwrap(),
            &0xffff_ffffu32.to_be_bytes(),
        );

        let wid_ptr: u32 = 0x6_0000;
        let pm: u32 = 0x6_1000;
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);

        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            wid_ptr as u64,
            pm as u64,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_AGAIN as u64,
            "all 32 bits set -> no free slot"
        );
    }

    #[test]
    fn add_workload_with_freed_slot_populates_wkl_info_1() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);

        // Clear wklEnabled to 0 so wid 0 is the next slot.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_ENABLED) as u64),
                4,
            )
            .unwrap(),
            &0u32.to_be_bytes(),
        );

        let wid_ptr: u32 = 0x6_0000;
        let pm: u32 = 0x6_1000;
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);

        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            wid_ptr as u64,
            pm as u64,
            0x1000,
            0xdead_beef,
            priority_ptr as u64,
            3,
            4,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0, "CELL_OK");

        assert_eq!(read_u32_be(&rt, wid_ptr), 0);
        let info_addr = spurs + SPURS_OFF_WKL_INFO_1; // wid 0
        assert_eq!(read_u64_be(&rt, info_addr + WI_OFF_ADDR), pm as u64);
        assert_eq!(read_u64_be(&rt, info_addr + WI_OFF_ARG), 0xdead_beef);
        assert_eq!(read_u32_be(&rt, info_addr + WI_OFF_SIZE), 0x1000);
        assert_eq!(read_byte_at(&rt, info_addr + WI_OFF_UNIQUE_ID), 0);
        assert_eq!(
            read_byte_at(&rt, spurs + SPURS_OFF_WKL_STATE_1),
            SPURS_WKL_STATE_RUNNABLE
        );
        assert_eq!(
            read_byte_at(&rt, spurs + SPURS_OFF_WKL_MIN_CONTENTION),
            3,
            "minCnt clamped to caller value"
        );
        // wklEnabled bit 0x80000000 set after enabling wid 0.
        assert_eq!(read_u32_be(&rt, spurs + SPURS_OFF_WKL_ENABLED), 0x8000_0000);
    }

    #[test]
    fn add_workload_rejects_misaligned_pm() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);

        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);

        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x6_0000,
            /* pm misaligned (8-byte not 16) = */ 0x6_1008,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_ALIGN as u64
        );
    }

    #[test]
    fn add_workload_rejects_zero_min_cnt() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);

        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);

        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x6_0000,
            0x6_1000,
            0x1000,
            0,
            priority_ptr as u64,
            /* minCnt = */ 0,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn add_workload_rejects_priority_out_of_range() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);

        let priority_ptr: u32 = 0x6_2000;
        // Plant priority byte > 15.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(priority_ptr as u64), 8)
                .unwrap(),
            &[0, 0, 0, 0x10, 0, 0, 0, 0],
        );

        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x6_0000,
            0x6_1000,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn shutdown_workload_transitions_runnable_to_shutting_down() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        // Clear wklEnabled, then add a workload at wid=0.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_ENABLED) as u64),
                4,
            )
            .unwrap(),
            &0u32.to_be_bytes(),
        );
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);
        let add_args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x6_0000,
            0x6_1000,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &add_args);
        let _ = drain_return(&mut rt, unit_id);

        // Shutdown wid=0.
        let sd_args: [u64; 9] = [0x10000, spurs as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SHUTDOWN_WORKLOAD, &sd_args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(
            read_byte_at(&rt, spurs + SPURS_OFF_WKL_STATE_1),
            SPURS_WKL_STATE_SHUTTING_DOWN
        );
        assert_eq!(
            read_byte_at(&rt, spurs + SPURS_OFF_WKL_EVENT_1) & 1,
            1,
            "shutdown sets event bit 0"
        );
    }

    #[test]
    fn shutdown_workload_invalid_wid_rejected() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 99, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SHUTDOWN_WORKLOAD, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn wait_for_workload_shutdown_validates_wid_and_enabled_bit() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        // wid 99 exceeds wmax=16 for SPURS1 -> INVAL.
        let args: [u64; 9] = [0x10000, spurs as u64, 99, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_WAIT_FOR_WORKLOAD_SHUTDOWN, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64
        );

        // wid 5: post-init wklEnabled is 0x0000_ffff (the SPURS1
        // reserved-low-band seed). The check bit for wid 5 is
        // (0x80000000 >> 5), which is in the high band and thus
        // unset, so the call returns SRCH.
        assert_eq!(
            read_u32_be(&rt, spurs + SPURS_OFF_WKL_ENABLED),
            0x0000_FFFF,
            "post-init wklEnabled is 0x0000_FFFF for SPURS1"
        );
        let args2: [u64; 9] = [0x10000, spurs as u64, 5, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_WAIT_FOR_WORKLOAD_SHUTDOWN, &args2);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_SRCH as u64
        );

        // After AddWorkload at wid=0, bit 0x80000000 is set -> CELL_OK.
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);
        let add_args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x6_0000,
            0x6_1000,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &add_args);
        let _ = drain_return(&mut rt, unit_id);

        let args3: [u64; 9] = [0x10000, spurs as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_WAIT_FOR_WORKLOAD_SHUTDOWN, &args3);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
    }

    #[test]
    fn initialize_bare_writes_flags1_with_exit_if_no_work_bit() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            /* nSpus = */ 2,
            /* spuPrio = */ 200,
            /* ppuPrio = */ 1000,
            /* exitIfNoWork = */ 1,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_INITIALIZE, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        // SF1_EXIT_IF_NO_WORK = 0x80, SF1_32_WORKLOADS = 0x40.
        assert_eq!(
            read_byte_at(&rt, spurs + SPURS_OFF_FLAGS1),
            0x80,
            "SPURS1 with exitIfNoWork should set flags1 = SF1_EXIT_IF_NO_WORK"
        );
    }

    #[test]
    fn initialize_with_attribute_v2_writes_flags1_with_32_workloads_bit() {
        let (mut rt, unit_id) = fixture();
        let attr_ptr: u32 = 0x2_0000;
        let spurs: u32 = 0x4_0000;
        // Plant attribute via _cellSpursAttributeInitialize.
        let attr_args: [u64; 9] = [0x10000, attr_ptr as u64, 2, 0, 1, 128, 1000, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ATTRIBUTE_INITIALIZE, &attr_args);
        let _ = drain_return(&mut rt, unit_id);

        let init_args: [u64; 9] = [0x10000, spurs as u64, attr_ptr as u64, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_INITIALIZE_WITH_ATTRIBUTE2,
            &init_args,
        );
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(
            read_byte_at(&rt, spurs + SPURS_OFF_FLAGS1) & 0x40,
            0x40,
            "WithAttribute2 should set flags1 SF1_32_WORKLOADS"
        );
    }

    #[test]
    fn initialize_rejects_n_spus_out_of_range() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        // n_spus = 0 (< 1).
        let args0: [u64; 9] = [0x10000, spurs as u64, 0, 200, 1000, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_INITIALIZE, &args0);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_INVAL as u64
        );
        // n_spus = 7 (> 6).
        let args7: [u64; 9] = [0x10000, spurs as u64, 7, 200, 1000, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_INITIALIZE, &args7);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn add_workload_returns_stat_when_exception_is_set() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        // Plant a non-zero exception so the next AddWorkload bails.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_EXCEPTION) as u64),
                4,
            )
            .unwrap(),
            &1u32.to_be_bytes(),
        );
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x6_0000,
            0x6_1000,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_STAT as u64
        );
    }

    #[test]
    fn add_workload_does_not_write_wid_when_full() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        // Saturate wklEnabled so AGAIN fires.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_ENABLED) as u64),
                4,
            )
            .unwrap(),
            &0xffff_ffffu32.to_be_bytes(),
        );

        // Sentinel must survive an AGAIN return (no out-pointer write
        // on the failure branch).
        let wid_ptr: u32 = 0x6_0000;
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(wid_ptr as u64), 4).unwrap(),
            &0xCAFEBABEu32.to_be_bytes(),
        );

        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            wid_ptr as u64,
            0x6_1000,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_AGAIN as u64
        );
        assert_eq!(
            read_u32_be(&rt, wid_ptr),
            0xCAFEBABE,
            "wid_ptr must stay unmodified on AGAIN"
        );
    }

    #[test]
    fn add_workload_fills_slots_in_high_bit_first_order() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        // Clear wklEnabled so the first 16 AddWorkload calls land
        // in wid 0..15 in order.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_ENABLED) as u64),
                4,
            )
            .unwrap(),
            &0u32.to_be_bytes(),
        );

        let wid_ptr: u32 = 0x6_0000;
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);

        for expected_wid in 0u32..16 {
            let args: [u64; 9] = [
                0x10000,
                spurs as u64,
                wid_ptr as u64,
                0x6_1000,
                0x1000,
                expected_wid as u64,
                priority_ptr as u64,
                1,
                2,
            ];
            rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
            assert_eq!(
                drain_return(&mut rt, unit_id),
                0,
                "AddWorkload {expected_wid} should succeed"
            );
            assert_eq!(read_u32_be(&rt, wid_ptr), expected_wid);
        }
        // wklEnabled was cleared to 0 above, so 16 successful
        // AddWorkloads set bits 31..16 in MSB-first order, leaving
        // 0xFFFF_0000.
        assert_eq!(
            read_u32_be(&rt, spurs + SPURS_OFF_WKL_ENABLED),
            0xFFFF_0000,
            "16 successful AddWorkloads set the top 16 bits"
        );
        // 17th call returns AGAIN.
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            wid_ptr as u64,
            0x6_1000,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_AGAIN as u64
        );
    }

    #[test]
    fn add_workload_with_attribute_routes_through_internal_path() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        // Clear wklEnabled so wid 0 is allocated.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_ENABLED) as u64),
                4,
            )
            .unwrap(),
            &0u32.to_be_bytes(),
        );

        // Plant a CellSpursWorkloadAttribute by hand: revision=1,
        // pm=0x6_1000, size=0x1000, data=0x1234, priority bytes 0..7,
        // minCnt=2, maxCnt=4.
        let attr_ptr: u32 = 0x2_0000;
        let mut attr_block = [0u8; 512];
        attr_block[..4].copy_from_slice(&1u32.to_be_bytes()); // revision
        attr_block[8..12].copy_from_slice(&0x6_1000u32.to_be_bytes()); // pm
        attr_block[12..16].copy_from_slice(&0x1000u32.to_be_bytes()); // size
        attr_block[16..24].copy_from_slice(&0x1234u64.to_be_bytes()); // data
        attr_block[24..32].copy_from_slice(&[0, 1, 2, 3, 4, 5, 6, 7]); // priority
        attr_block[32..36].copy_from_slice(&2u32.to_be_bytes()); // minCnt
        attr_block[36..40].copy_from_slice(&4u32.to_be_bytes()); // maxCnt
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(attr_ptr as u64), 512).unwrap(),
            &attr_block,
        );

        let wid_ptr: u32 = 0x6_0000;
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            wid_ptr as u64,
            attr_ptr as u64,
            0,
            0,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD_WITH_ATTRIBUTE, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(read_u32_be(&rt, wid_ptr), 0);
        let info_addr = spurs + SPURS_OFF_WKL_INFO_1;
        assert_eq!(read_u64_be(&rt, info_addr + WI_OFF_ADDR), 0x6_1000);
        assert_eq!(read_u64_be(&rt, info_addr + WI_OFF_ARG), 0x1234);
        assert_eq!(read_u32_be(&rt, info_addr + WI_OFF_SIZE), 0x1000);
    }

    #[test]
    fn add_workload_with_attribute_rejects_revision_two() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);

        let attr_ptr: u32 = 0x2_0000;
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(attr_ptr as u64), 4).unwrap(),
            &2u32.to_be_bytes(),
        );

        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x6_0000,
            attr_ptr as u64,
            0,
            0,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD_WITH_ATTRIBUTE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64
        );
    }

    /// SPURS2 mirror -- the exception field is at the same 0xD6C
    /// offset; a refactor that moves it inside the SPURS2 bank by
    /// mistake fails here while the SPURS1 sibling still passes.
    #[test]
    fn add_workload_returns_stat_when_exception_is_set_v2() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        let attr_ptr: u32 = 0x2_0000;
        init_spurs_v2(&mut rt, unit_id, spurs, attr_ptr);
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_EXCEPTION) as u64),
                4,
            )
            .unwrap(),
            &1u32.to_be_bytes(),
        );
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x6_0000,
            0x6_1000,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_STAT as u64
        );
    }

    /// Exercises the SPURS2 wid >= 16 branch (wklInfo2 indexing,
    /// bank-selection debug_assert) that the SPURS1 tests never reach.
    #[test]
    fn add_workload_fills_slots_in_high_bit_first_order_v2() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        let attr_ptr: u32 = 0x2_0000;
        init_spurs_v2(&mut rt, unit_id, spurs, attr_ptr);

        // Clear wklEnabled so the first 32 calls land in MSB-first
        // order. SPURS2 init does not seed the reserved band, so this
        // is defensive.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_ENABLED) as u64),
                4,
            )
            .unwrap(),
            &0u32.to_be_bytes(),
        );

        let wid_ptr: u32 = 0x6_0000;
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);

        for expected_wid in 0u32..32 {
            let args: [u64; 9] = [
                0x10000,
                spurs as u64,
                wid_ptr as u64,
                0x6_1000,
                0x1000,
                expected_wid as u64,
                priority_ptr as u64,
                1,
                2,
            ];
            rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
            assert_eq!(
                drain_return(&mut rt, unit_id),
                0,
                "AddWorkload {expected_wid} should succeed under SPURS2"
            );
            assert_eq!(read_u32_be(&rt, wid_ptr), expected_wid);
        }
        // 32 successful AddWorkloads set every bit.
        assert_eq!(
            read_u32_be(&rt, spurs + SPURS_OFF_WKL_ENABLED),
            0xFFFF_FFFF,
            "32 successful AddWorkloads exhaust SPURS2's wid space"
        );
        // wid 0 went to wklInfo1 at offset 0xB00.
        let info_addr_0 = spurs + SPURS_OFF_WKL_INFO_1;
        assert_eq!(
            read_u64_be(&rt, info_addr_0 + WI_OFF_ADDR),
            0x6_1000,
            "wid 0's pm landed in wklInfo1[0]"
        );
        // wid 16 went to wklInfo2 at offset 0x1000 (start of bank
        // 2, inside the 8 KiB SPURS2 region).
        let info_addr_16 = spurs + SPURS_OFF_WKL_INFO_2;
        assert_eq!(
            read_u64_be(&rt, info_addr_16 + WI_OFF_ADDR),
            0x6_1000,
            "wid 16's pm landed in wklInfo2[0]"
        );
        // 33rd call returns AGAIN.
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            wid_ptr as u64,
            0x6_1000,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_AGAIN as u64
        );
    }

    /// Init SPURS1 and add one workload at wid 0 (RUNNABLE).
    fn fixture_with_one_workload(spurs: u32) -> (Runtime, UnitId) {
        let (mut rt, unit_id) = fixture();
        init_spurs(&mut rt, unit_id, spurs);
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_ENABLED) as u64),
                4,
            )
            .unwrap(),
            &0u32.to_be_bytes(),
        );
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x6_0000,
            0x6_1000,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        let _ = drain_return(&mut rt, unit_id);
        (rt, unit_id)
    }

    #[test]
    fn ready_count_store_writes_byte() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 0x42, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_READY_COUNT_STORE, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(read_byte_at(&rt, spurs + SPURS_OFF_WKL_READY_COUNT_1), 0x42);
    }

    #[test]
    fn ready_count_store_rejects_value_above_0xff() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 0x100, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_READY_COUNT_STORE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn ready_count_store_rejects_invalid_wid() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        // wid = 0xffffffff is past wmax -> INVAL.
        let args: [u64; 9] = [0x10000, spurs as u64, 0xffff_ffff, 1, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_READY_COUNT_STORE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn ready_count_store_rejects_disabled_wid() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        // wid 5 is in band but not enabled (only wid 0 was added).
        let args: [u64; 9] = [0x10000, spurs as u64, 5, 1, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_READY_COUNT_STORE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_SRCH as u64
        );
    }

    #[test]
    fn ready_count_store_rejects_non_runnable() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        // Demote wid 0 from RUNNABLE to PREPARING via direct write.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_STATE_1) as u64),
                1,
            )
            .unwrap(),
            &[SPURS_WKL_STATE_PREPARING],
        );
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 1, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_READY_COUNT_STORE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_STAT as u64
        );
    }

    #[test]
    fn ready_count_swap_returns_prior_through_out_pointer() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        // Plant readyCount = 0x33.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_READY_COUNT_1) as u64),
                1,
            )
            .unwrap(),
            &[0x33],
        );
        let old_ptr: u32 = 0x7_0000;
        let args: [u64; 9] = [0x10000, spurs as u64, 0, old_ptr as u64, 0x77, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_READY_COUNT_SWAP, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(read_u32_be(&rt, old_ptr), 0x33);
        assert_eq!(read_byte_at(&rt, spurs + SPURS_OFF_WKL_READY_COUNT_1), 0x77);
    }

    #[test]
    fn ready_count_compare_and_swap_only_swaps_on_match() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_READY_COUNT_1) as u64),
                1,
            )
            .unwrap(),
            &[0x10],
        );
        let old_ptr: u32 = 0x7_0000;
        // Mismatched compare: prior = 0x10, compare = 0x99 -> no swap.
        let args_no_swap: [u64; 9] = [
            0x10000,
            spurs as u64,
            0,
            old_ptr as u64,
            0x99,
            0x55,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_READY_COUNT_COMPARE_AND_SWAP,
            &args_no_swap,
        );
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(read_u32_be(&rt, old_ptr), 0x10);
        assert_eq!(
            read_byte_at(&rt, spurs + SPURS_OFF_WKL_READY_COUNT_1),
            0x10,
            "no swap on compare mismatch"
        );

        // Matched compare: 0x10 == 0x10 -> swap to 0x55.
        let args_swap: [u64; 9] = [
            0x10000,
            spurs as u64,
            0,
            old_ptr as u64,
            0x10,
            0x55,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_READY_COUNT_COMPARE_AND_SWAP,
            &args_swap,
        );
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(read_u32_be(&rt, old_ptr), 0x10);
        assert_eq!(read_byte_at(&rt, spurs + SPURS_OFF_WKL_READY_COUNT_1), 0x55);
    }

    #[test]
    fn ready_count_add_clamps_and_returns_prior() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_READY_COUNT_1) as u64),
                1,
            )
            .unwrap(),
            &[0xF0],
        );
        let old_ptr: u32 = 0x7_0000;
        // 0xF0 + 0x20 = 0x110 -> clamped to 0xFF.
        let args: [u64; 9] = [0x10000, spurs as u64, 0, old_ptr as u64, 0x20, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_READY_COUNT_ADD, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(read_u32_be(&rt, old_ptr), 0xF0);
        assert_eq!(read_byte_at(&rt, spurs + SPURS_OFF_WKL_READY_COUNT_1), 0xFF);
    }

    #[test]
    fn ready_count_swap_null_old_rejected() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 0, 0x55, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_READY_COUNT_SWAP, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER as u64
        );
    }

    #[test]
    fn request_idle_spu_writes_idle_count_for_valid_wid() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 4, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_REQUEST_IDLE_SPU, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(
            read_byte_at(&rt, spurs + SPURS_OFF_WKL_IDLE_SPU_COUNT_OR_RC2),
            4
        );
    }

    #[test]
    fn request_idle_spu_rejects_spurs2() {
        let spurs: u32 = 0x4_0000;
        let attr_ptr: u32 = 0x2_0000;
        let mut rt = fixture().0;
        let unit_id = UnitId::new(0);
        init_spurs_v2(&mut rt, unit_id, spurs, attr_ptr);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 4, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_REQUEST_IDLE_SPU, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_STAT as u64,
            "RequestIdleSpu does not support 32-workloads (SF1_32_WORKLOADS) -> STAT"
        );
    }

    #[test]
    fn request_idle_spu_rejects_broadcast_sentinel_wid() {
        // wid = 0xffffffff is past MAX_WORKLOAD -> INVAL.
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0xffff_ffff, 7, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_REQUEST_IDLE_SPU, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn request_idle_spu_rejects_count_at_or_above_max_spu() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 8, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_REQUEST_IDLE_SPU, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn set_max_contention_writes_low_nibble_for_wid_under_16() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        // AddWorkload seeded the low nibble to 2; overwrite to 5.
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 5, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SET_MAX_CONTENTION, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        let mc = read_byte_at(&rt, spurs + SPURS_OFF_WKL_MAX_CONTENTION);
        assert_eq!(mc & 0x0f, 5, "low nibble holds the SPURS1 wid 0..15 value");
    }

    #[test]
    fn set_max_contention_clamps_to_max_spu() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 99, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SET_MAX_CONTENTION, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        let mc = read_byte_at(&rt, spurs + SPURS_OFF_WKL_MAX_CONTENTION);
        assert_eq!(
            mc & 0x0f,
            CELL_SPURS_MAX_SPU as u8,
            "values above CELL_SPURS_MAX_SPU clamp to 8"
        );
    }

    #[test]
    fn set_priorities_writes_8_byte_table() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let prio_ptr: u32 = 0x7_0000;
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(prio_ptr as u64), 8).unwrap(),
            &[1, 2, 3, 4, 5, 6, 7, 8],
        );
        let args: [u64; 9] = [0x10000, spurs as u64, 0, prio_ptr as u64, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SET_PRIORITIES, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        let info_addr = spurs + SPURS_OFF_WKL_INFO_1;
        let mem = rt.memory().as_bytes();
        let written = &mem
            [(info_addr + WI_OFF_PRIORITY) as usize..(info_addr + WI_OFF_PRIORITY + 8) as usize];
        assert_eq!(written, &[1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn set_priorities_rejects_priority_above_15() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let prio_ptr: u32 = 0x7_0000;
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(prio_ptr as u64), 8).unwrap(),
            &[0, 0, 0x10, 0, 0, 0, 0, 0],
        );
        let args: [u64; 9] = [0x10000, spurs as u64, 0, prio_ptr as u64, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SET_PRIORITIES, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn set_priority_writes_single_byte() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        // init_spurs picks nSpus=1.
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 0, 7, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SET_PRIORITY, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        let info_addr = spurs + SPURS_OFF_WKL_INFO_1;
        assert_eq!(read_byte_at(&rt, info_addr + WI_OFF_PRIORITY), 7);
    }

    #[test]
    fn set_priority_rejects_priority_at_max() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 0, 16, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SET_PRIORITY, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn set_priority_rejects_spu_id_at_or_above_n_spus() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        // nSpus = 1 (set by init_spurs), so spu_id = 1 is OOB.
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 1, 5, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SET_PRIORITY, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_INVAL as u64
        );
    }

    /// Debug-build half of the fail-loud contract on
    /// `_cellSpursWorkloadAttributeInitialize`.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "maxCnt")]
    fn workload_attribute_initialize_panics_until_spill_surface_lands() {
        let (mut rt, unit_id) = fixture();
        let args: [u64; 9] = [0x10000, 0x4_0000, 1, 0, 0x6_1000, 0x1000, 0, 0x6_2000, 2];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_WORKLOAD_ATTRIBUTE_INITIALIZE, &args);
    }

    #[cfg(not(debug_assertions))]
    #[test]
    fn workload_attribute_initialize_returns_inval_in_release() {
        let (mut rt, unit_id) = fixture();
        let args: [u64; 9] = [0x10000, 0x4_0000, 1, 0, 0x6_1000, 0x1000, 0, 0x6_2000, 2];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_WORKLOAD_ATTRIBUTE_INITIALIZE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn get_info_writes_basic_fields_after_init() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        let info: u32 = 0x6_0000;
        // SPURS1 init: nSpus=2, spuPrio=250, ppuPrio=1000, exitIfNoWork=1.
        let init: [u64; 9] = [0x10000, spurs as u64, 2, 250, 1000, 1, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_INITIALIZE, &init);
        let _ = drain_return(&mut rt, unit_id);

        let args: [u64; 9] = [0x10000, spurs as u64, info as u64, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_GET_INFO, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);

        assert_eq!(read_u32_be(&rt, info + INFO_OFF_NSPUS), 2);
        assert_eq!(
            read_u32_be(&rt, info + INFO_OFF_SPU_THREAD_GROUP_PRIORITY),
            250
        );
        assert_eq!(read_u32_be(&rt, info + INFO_OFF_PPU_THREAD_PRIORITY), 1000);
        assert_eq!(read_byte_at(&rt, info + INFO_OFF_EXIT_IF_NO_WORK), 1);
        assert_eq!(read_byte_at(&rt, info + INFO_OFF_SPURS2), 0, "SPURS1");
        assert_eq!(
            read_u64_be(&rt, info + INFO_OFF_SPURS_HANDLER_THREAD_0),
            u64::MAX,
            "ppu0 init sentinel"
        );
    }

    #[test]
    fn get_info_v2_sets_spurs2_byte() {
        let (mut rt, unit_id) = fixture();
        let attr_ptr: u32 = 0x2_0000;
        let spurs: u32 = 0x4_0000;
        let info: u32 = 0x6_0000;
        init_spurs_v2(&mut rt, unit_id, spurs, attr_ptr);

        let args: [u64; 9] = [0x10000, spurs as u64, info as u64, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_GET_INFO, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(read_byte_at(&rt, info + INFO_OFF_SPURS2), 1);
    }

    #[test]
    fn get_info_null_info_rejected() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        let init: [u64; 9] = [0x10000, spurs as u64, 1, 250, 1000, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_INITIALIZE, &init);
        let _ = drain_return(&mut rt, unit_id);

        let args: [u64; 9] = [0x10000, spurs as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_GET_INFO, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_NULL_POINTER as u64
        );
    }

    #[test]
    fn get_info_misaligned_spurs_rejected() {
        let (mut rt, unit_id) = fixture();
        let args: [u64; 9] = [0x10000, 0x4_0040, 0x6_0000, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_GET_INFO, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_ALIGN as u64
        );
    }

    #[test]
    fn attach_lv2_event_queue_static_writes_event_port_mux() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        let port_ptr: u32 = 0x6_0000;
        init_spurs(&mut rt, unit_id, spurs);
        // Plant *port = 0x0c.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(port_ptr as u64), 1).unwrap(),
            &[0x0c],
        );

        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            /* queue = */ 0x4000_0019,
            port_ptr as u64,
            /* isDynamic = */ 0,
            0,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ATTACH_LV2_EVENT_QUEUE, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);

        assert_eq!(
            read_u32_be(&rt, spurs + SPURS_OFF_EVENT_PORT_MUX + EPM_OFF_SPU_PORT),
            0x0c
        );
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_EVENT_PORT_MUX + EPM_OFF_EVENT_PORT),
            0x4000_0019
        );
        // spuPortBits gains bit 0x0c.
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_SPU_PORT_BITS),
            1u64 << 0x0c
        );
    }

    #[test]
    fn attach_lv2_event_queue_dynamic_allocates_first_unused_port() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        let port_ptr: u32 = 0x6_0000;
        init_spurs(&mut rt, unit_id, spurs);

        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x4000_0019,
            port_ptr as u64,
            /* isDynamic = */ 1,
            0,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ATTACH_LV2_EVENT_QUEUE, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        // Dynamic allocator picks the lowest free bit in [0x10, 0x40);
        // post-init spuPortBits is zero, so port 0x10 is chosen.
        assert_eq!(read_byte_at(&rt, port_ptr), 0x10);
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_SPU_PORT_BITS),
            1u64 << 0x10
        );
    }

    #[test]
    fn attach_lv2_event_queue_static_rejects_port_above_3f() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        let port_ptr: u32 = 0x6_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(port_ptr as u64), 1).unwrap(),
            &[0x40],
        );
        let args: [u64; 9] = [0x10000, spurs as u64, 0, port_ptr as u64, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ATTACH_LV2_EVENT_QUEUE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn attach_lv2_event_queue_null_port_pointer_rejected() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ATTACH_LV2_EVENT_QUEUE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_NULL_POINTER as u64
        );
    }

    #[test]
    fn detach_lv2_event_queue_clears_bit_when_set() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        let port_ptr: u32 = 0x6_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(port_ptr as u64), 1).unwrap(),
            &[0x0c],
        );
        let attach_args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x4000_0019,
            port_ptr as u64,
            0,
            0,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ATTACH_LV2_EVENT_QUEUE, &attach_args);
        let _ = drain_return(&mut rt, unit_id);

        let detach: [u64; 9] = [0x10000, spurs as u64, 0x0c, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_DETACH_LV2_EVENT_QUEUE, &detach);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(read_u64_be(&rt, spurs + SPURS_OFF_SPU_PORT_BITS), 0);
        // EventPortMux::eventPort cleared because the detached port
        // matches the bound spuPort.
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_EVENT_PORT_MUX + EPM_OFF_EVENT_PORT),
            0
        );
    }

    #[test]
    fn detach_lv2_event_queue_returns_srch_for_clear_bit() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let detach: [u64; 9] = [0x10000, spurs as u64, 0x05, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_DETACH_LV2_EVENT_QUEUE, &detach);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_SRCH as u64
        );
    }

    #[test]
    fn detach_lv2_event_queue_rejects_port_above_3f() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let detach: [u64; 9] = [0x10000, spurs as u64, 0x40, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_DETACH_LV2_EVENT_QUEUE, &detach);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn set_global_exception_event_handler_writes_handler_and_args() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            /* eaHandler = */ 0x6_1000,
            /* arg = */ 0x6_2000,
            0,
            0,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_SET_GLOBAL_EXCEPTION_EVENT_HANDLER,
            &args,
        );
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER),
            0x6_1000
        );
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER_ARGS),
            0x6_2000
        );
    }

    #[test]
    fn set_global_exception_event_handler_returns_busy_when_already_set() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0x6_1000, 0x6_2000, 0, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_SET_GLOBAL_EXCEPTION_EVENT_HANDLER,
            &args,
        );
        let _ = drain_return(&mut rt, unit_id);
        // Second registration without unset returns BUSY.
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_SET_GLOBAL_EXCEPTION_EVENT_HANDLER,
            &args,
        );
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_BUSY as u64
        );
    }

    #[test]
    fn unset_global_exception_event_handler_clears_both_slots() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let set_args: [u64; 9] = [0x10000, spurs as u64, 0x6_1000, 0x6_2000, 0, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_SET_GLOBAL_EXCEPTION_EVENT_HANDLER,
            &set_args,
        );
        let _ = drain_return(&mut rt, unit_id);

        let unset_args: [u64; 9] = [0x10000, spurs as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_UNSET_GLOBAL_EXCEPTION_EVENT_HANDLER,
            &unset_args,
        );
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER),
            0
        );
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER_ARGS),
            0
        );
    }

    #[test]
    fn set_exception_event_handler_routes_global_sentinel_wid() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        // The 0xffffffff sentinel routes to the global-handler slot.
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0xffff_ffff,
            /* hook = */ 0x6_3000,
            /* taskset = */ 0x6_4000,
            0,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SET_EXCEPTION_EVENT_HANDLER, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER),
            0x6_3000
        );
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER_ARGS),
            0x6_4000
        );
    }

    #[test]
    fn set_exception_event_handler_per_workload_returns_ok_no_global_write() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 0x6_3000, 0x6_4000, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SET_EXCEPTION_EVENT_HANDLER, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        // Global slot must stay zero: per-workload path is a noop.
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER),
            0
        );
    }

    #[test]
    fn set_exception_event_handler_per_workload_invalid_wid_rejected() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        // wid=20 is out of band (wmax=16) and not the sentinel.
        let args: [u64; 9] = [0x10000, spurs as u64, 20, 0x6_3000, 0x6_4000, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SET_EXCEPTION_EVENT_HANDLER, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_INVAL as u64
        );
    }

    #[test]
    fn unset_exception_event_handler_global_sentinel_clears_both_slots() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let set_args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0xffff_ffff,
            0x6_3000,
            0x6_4000,
            0,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_SET_EXCEPTION_EVENT_HANDLER,
            &set_args,
        );
        let _ = drain_return(&mut rt, unit_id);
        let unset_args: [u64; 9] = [0x10000, spurs as u64, 0xffff_ffff, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_UNSET_EXCEPTION_EVENT_HANDLER,
            &unset_args,
        );
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER),
            0
        );
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER_ARGS),
            0
        );
    }

    #[test]
    fn enable_exception_event_handler_writes_one_when_flag_set() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 1, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_ENABLE_EXCEPTION_EVENT_HANDLER,
            &args,
        );
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(read_u32_be(&rt, spurs + SPURS_OFF_ENABLE_EH), 1);
    }

    #[test]
    fn enable_exception_event_handler_writes_zero_when_flag_clear() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        // Pre-seed enableEH = 1 then disable.
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_ENABLE_EH) as u64),
                4,
            )
            .unwrap(),
            &1u32.to_be_bytes(),
        );
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_ENABLE_EXCEPTION_EVENT_HANDLER,
            &args,
        );
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(read_u32_be(&rt, spurs + SPURS_OFF_ENABLE_EH), 0);
    }

    /// 8 MiB main region; an attr above that must surface as a real
    /// read failure, not zero-substitution that falsely accepts an
    /// uninitialised revision.
    #[test]
    fn initialize_with_attribute_unmapped_attr_rejected_with_inval() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        // 0x10_0000_00 (256 MiB) is well past the 8 MiB fixture.
        let unmapped_attr: u32 = 0x1000_0000;
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            unmapped_attr as u64,
            0,
            0,
            0,
            0,
            0,
            0,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_INITIALIZE_WITH_ATTRIBUTE, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_CORE_ERROR_INVAL as u64,
            "an unmapped attr must surface as INVAL, not silent CELL_OK with a zero attribute"
        );
    }

    /// AddWorkload should report FAULT (not silent success or INVAL)
    /// when the spurs pointer doesn't land inside any region.
    #[test]
    fn add_workload_unmapped_spurs_returns_fault() {
        let (mut rt, unit_id) = fixture();
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);
        let unmapped_spurs: u32 = 0x1000_0000;
        let args: [u64; 9] = [
            0x10000,
            unmapped_spurs as u64,
            0x6_0000,
            0x6_1000,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_FAULT as u64,
        );
    }

    /// `priority_ptr == 0` must surface as NULL_POINTER, not INVAL.
    #[test]
    fn add_workload_null_priority_pointer_returns_null_pointer() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x6_0000,
            0x6_1000,
            0x1000,
            0,
            /* priority_ptr = */ 0,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(
            drain_return(&mut rt, unit_id),
            CELL_SPURS_POLICY_MODULE_ERROR_NULL_POINTER as u64,
        );
    }

    /// Two AddWorkloads with the same `pm` reuse uniqueId.
    #[test]
    fn add_workload_dedupes_unique_id_on_matching_pm() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_ENABLED) as u64),
                4,
            )
            .unwrap(),
            &0u32.to_be_bytes(),
        );
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);
        let pm: u32 = 0x6_1000;

        // wid 0: pm = 0x6_1000 -> uniqueId = 0.
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x6_0000,
            pm as u64,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);

        // wid 1: same pm -> uniqueId reused = 0.
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);

        let info_addr_1 = spurs + SPURS_OFF_WKL_INFO_1 + 32; // wid 1
        assert_eq!(
            read_byte_at(&rt, info_addr_1 + WI_OFF_UNIQUE_ID),
            0,
            "duplicate-pm AddWorkload reuses wid 0's uniqueId"
        );
    }

    /// AddWorkload pins the `wklMskB` bit-on-alloc polarity. RPCS3
    /// `_spurs::add_workload` sets `(0x80000000 >> wid)` (verified
    /// against `cellSpurs.cpp` ~line 2511).
    #[test]
    fn add_workload_sets_wkl_msk_b_bit_on_alloc() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        init_spurs(&mut rt, unit_id, spurs);
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_ENABLED) as u64),
                4,
            )
            .unwrap(),
            &0u32.to_be_bytes(),
        );
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);
        let args: [u64; 9] = [
            0x10000,
            spurs as u64,
            0x6_0000,
            0x6_1000,
            0x1000,
            0,
            priority_ptr as u64,
            1,
            2,
        ];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_ADD_WORKLOAD, &args);
        let _ = drain_return(&mut rt, unit_id);
        assert_eq!(
            read_u32_be(&rt, spurs + SPURS_OFF_WKL_MSK_B),
            0x8000_0000,
            "wid 0 alloc sets bit 0x80000000 in wklMskB",
        );
    }

    #[test]
    fn initialize_accepts_n_spus_at_one_and_six() {
        let (mut rt, unit_id) = fixture();
        let spurs: u32 = 0x4_0000;
        for &n in &[1i32, 6] {
            let args: [u64; 9] = [0x10000, spurs as u64, n as u64, 200, 1000, 0, 0, 0, 0];
            rt.dispatch_hle(unit_id, NID_CELL_SPURS_INITIALIZE, &args);
            assert_eq!(
                drain_return(&mut rt, unit_id),
                0,
                "n_spus = {n} (boundary) should pass init",
            );
            assert_eq!(read_byte_at(&rt, spurs + SPURS_OFF_NSPUS), n as u8);
        }
    }

    #[test]
    fn request_idle_spu_accepts_count_at_seven() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 7, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_REQUEST_IDLE_SPU, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(
            read_byte_at(&rt, spurs + SPURS_OFF_WKL_IDLE_SPU_COUNT_OR_RC2),
            7,
        );
    }

    #[test]
    fn set_priority_accepts_priority_at_fifteen() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 0, 15, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SET_PRIORITY, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        let info_addr = spurs + SPURS_OFF_WKL_INFO_1;
        assert_eq!(read_byte_at(&rt, info_addr + WI_OFF_PRIORITY), 15);
    }

    /// Wait-for-shutdown after Shutdown must remain a no-block CELL_OK
    /// path (no SPU kernel = no completion event ever fires).
    #[test]
    fn wait_for_workload_shutdown_after_shutdown_returns_ok() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let sd_args: [u64; 9] = [0x10000, spurs as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SHUTDOWN_WORKLOAD, &sd_args);
        let _ = drain_return(&mut rt, unit_id);
        let wait_args: [u64; 9] = [0x10000, spurs as u64, 0, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(
            unit_id,
            NID_CELL_SPURS_WAIT_FOR_WORKLOAD_SHUTDOWN,
            &wait_args,
        );
        assert_eq!(drain_return(&mut rt, unit_id), 0);
    }

    /// Per-workload `set_exception_event_handler` is a CELL_OK no-op:
    /// confirm both the global slot AND the per-workload state byte
    /// stay untouched.
    #[test]
    fn set_exception_event_handler_per_workload_leaves_state_untouched() {
        let spurs: u32 = 0x4_0000;
        let (mut rt, unit_id) = fixture_with_one_workload(spurs);
        let pre_status = read_byte_at(&rt, spurs + SPURS_OFF_WKL_STATUS_1);
        let pre_state = read_byte_at(&rt, spurs + SPURS_OFF_WKL_STATE_1);
        let args: [u64; 9] = [0x10000, spurs as u64, 0, 0x6_3000, 0x6_4000, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SET_EXCEPTION_EVENT_HANDLER, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(
            read_u64_be(&rt, spurs + SPURS_OFF_GLOBAL_EXCEPTION_HANDLER),
            0
        );
        assert_eq!(
            read_byte_at(&rt, spurs + SPURS_OFF_WKL_STATUS_1),
            pre_status
        );
        assert_eq!(read_byte_at(&rt, spurs + SPURS_OFF_WKL_STATE_1), pre_state);
    }

    /// SPURS2 path through `shutdown_workload`: `wid = 25` is in band
    /// only because `flags1 & SF1_32_WORKLOADS` -- pins the wmax
    /// derivation against any future hardcoded constant.
    #[test]
    fn shutdown_workload_v2_accepts_wid_in_spurs2_band() {
        let (mut rt, unit_id) = fixture();
        let attr_ptr: u32 = 0x2_0000;
        let spurs: u32 = 0x4_0000;
        init_spurs_v2(&mut rt, unit_id, spurs, attr_ptr);
        let priority_ptr: u32 = 0x6_2000;
        plant_priority_zero(&mut rt, priority_ptr);
        // Plant a workload at wid 25 (top bit pattern (0x80000000 >> 25)).
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_ENABLED) as u64),
                4,
            )
            .unwrap(),
            &(0x8000_0000u32 >> 25).to_be_bytes(),
        );
        // Plant state RUNNABLE for wid 25 -> wklState2[25 & 0xf] = wklState2[9].
        let _ = rt.memory_mut().apply_commit(
            cellgov_mem::ByteRange::new(
                cellgov_mem::GuestAddr::new((spurs + SPURS_OFF_WKL_STATE_2 + 9) as u64),
                1,
            )
            .unwrap(),
            &[SPURS_WKL_STATE_RUNNABLE],
        );
        let args: [u64; 9] = [0x10000, spurs as u64, 25, 0, 0, 0, 0, 0, 0];
        rt.dispatch_hle(unit_id, NID_CELL_SPURS_SHUTDOWN_WORKLOAD, &args);
        assert_eq!(drain_return(&mut rt, unit_id), 0);
        assert_eq!(
            read_byte_at(&rt, spurs + SPURS_OFF_WKL_STATE_2 + 9),
            SPURS_WKL_STATE_SHUTTING_DOWN,
        );
    }
}

#[cfg(test)]
mod canary_tests {
    use super::{dispatch, OWNED_NIDS};
    use crate::runtime::Runtime;
    use cellgov_event::UnitId;
    use cellgov_exec::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    fn canary_runtime() -> (Runtime, UnitId) {
        let mut rt = Runtime::new(GuestMemory::new(0x80_0000), Budget::new(1), 100);
        let unit_id = UnitId::new(0);
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.set_hle_heap_base(0x10_0000);
        (rt, unit_id)
    }

    /// Drift canary: every entry in [`OWNED_NIDS`] must be claimed by
    /// [`dispatch`]. Synthetic-zero args trip handler null-pointer
    /// guards; the panic from `_cellSpursWorkloadAttributeInitialize`'s
    /// fail-loud `debug_assert!` is caught here as evidence that
    /// routing succeeded before the body fired.
    #[test]
    fn owned_nids_all_claimed_by_dispatch() {
        for &nid in OWNED_NIDS {
            let (mut rt, unit_id) = canary_runtime();
            let args: [u64; 9] = [0; 9];
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                dispatch(&mut rt, unit_id, nid, &args)
            }));
            match outcome {
                Ok(Some(())) => {}
                Ok(None) => panic!(
                    "spurs::dispatch returned None for NID {nid:#010x} listed in \
                     OWNED_NIDS -- the match arm was likely removed without trimming \
                     the list"
                ),
                Err(_) => {
                    // Routing reached a handler body, then panicked.
                }
            }
        }
    }

    /// NIDs owned by other modules (and a synthetic 0xDEAD_BEEF) must
    /// return `None` so the dispatcher chain can keep walking.
    #[test]
    fn unowned_nids_are_rejected_by_dispatch() {
        let probes: &[u32] = &[
            crate::hle::cell_gcm_sys::NID_CELLGCM_INIT_BODY,
            crate::hle::sys_prx_for_user::NID_SYS_PPU_THREAD_GET_ID,
            0xDEAD_BEEF,
        ];
        for &nid in probes {
            let (mut rt, unit_id) = canary_runtime();
            let args: [u64; 9] = [0; 9];
            let result = dispatch(&mut rt, unit_id, nid, &args);
            assert_eq!(
                result, None,
                "spurs::dispatch claimed NID {nid:#010x} that is not in its OWNED_NIDS"
            );
        }
    }
}
