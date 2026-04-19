//! cellGcmSys HLE implementations.
//!
//! RSX graphics library (RPCS3 `Modules/cellGcmSys.cpp`). Real
//! behavior is gated behind the runtime's RSX-checkpoint flag; if
//! the flag is off, [`dispatch`] returns `None` so the router
//! falls through to the default CELL_OK fallback.

use cellgov_event::UnitId;

use crate::hle::context::{HleContext, RuntimeHleAdapter};
use crate::runtime::Runtime;

pub(crate) const NID_CELLGCM_GET_TILED_PITCH_SIZE: u32 = 0x055bd74d;
pub(crate) const NID_CELLGCM_INIT_BODY: u32 = 0x15bae46b;
pub(crate) const NID_CELLGCM_GET_CONFIGURATION: u32 = 0xe315a0b2;
pub(crate) const NID_CELLGCM_GET_CONTROL_REGISTER: u32 = 0xa547adde;
pub(crate) const NID_CELLGCM_GET_LABEL_ADDRESS: u32 = 0xf80196c1;

/// Per-module state for cellGcmSys. Owned by the HLE dispatch layer,
/// not by Runtime.
#[derive(Debug, Default)]
pub(crate) struct GcmState {
    pub(crate) context_addr: u32,
    pub(crate) control_addr: u32,
    pub(crate) io_address: u32,
    pub(crate) io_size: u32,
    pub(crate) local_size: u32,
    pub(crate) label_addr: u32,
    pub(crate) rsx_checkpoint: bool,
}

pub(crate) const TILED_PITCHES: &[u32] = &[
    0x000, 0x200, 0x300, 0x400, 0x500, 0x600, 0x700, 0x800, 0xA00, 0xC00, 0xD00, 0xE00, 0x1000,
    0x1400, 0x1800, 0x1A00, 0x1C00, 0x2000, 0x2800, 0x3000, 0x3400, 0x3800, 0x4000, 0x5000, 0x6000,
    0x6800, 0x7000, 0x8000, 0xA000, 0xC000, 0xD000, 0xE000, 0x10000,
];

/// Dispatch entry point for cellGcmSys handlers.
///
/// Returns `None` when either (a) the NID is not owned by this
/// module or (b) the runtime's RSX checkpoint is not enabled --
/// disabling at the dispatch boundary keeps non-RSX boot paths
/// on the default CELL_OK fallback instead of accidentally
/// exercising the RSX stubs.
pub(crate) fn dispatch(
    runtime: &mut Runtime,
    source: UnitId,
    nid: u32,
    args: &[u64; 9],
) -> Option<()> {
    if !runtime.gcm_state.rsx_checkpoint {
        return None;
    }
    match nid {
        NID_CELLGCM_GET_TILED_PITCH_SIZE => {
            get_tiled_pitch_size(&mut adapter(runtime, source), args);
        }
        NID_CELLGCM_INIT_BODY => {
            init_body_with_runtime(runtime, source, args);
        }
        NID_CELLGCM_GET_CONFIGURATION => {
            get_configuration_with_runtime(runtime, source, args);
        }
        NID_CELLGCM_GET_CONTROL_REGISTER => {
            let ctrl = runtime.gcm_state.control_addr as u64;
            adapter(runtime, source).set_return(ctrl);
        }
        NID_CELLGCM_GET_LABEL_ADDRESS => {
            let index = args[1] as u32;
            let addr = (runtime.gcm_state.label_addr + 0x10 * index) as u64;
            adapter(runtime, source).set_return(addr);
        }
        _ => return None,
    }
    Some(())
}

fn adapter(runtime: &mut Runtime, source: UnitId) -> RuntimeHleAdapter<'_> {
    RuntimeHleAdapter {
        memory: &mut runtime.memory,
        registry: &mut runtime.registry,
        heap_ptr: &mut runtime.hle_heap_ptr,
        next_id: &mut runtime.hle_next_id,
        source,
    }
}

fn init_body_with_runtime(runtime: &mut Runtime, source: UnitId, args: &[u64; 9]) {
    // Split-borrow: gcm_state and the ctx fields sit on Runtime;
    // construct the adapter first, mutate state via the returned
    // ctx's surrounding scope. Done inline here because both
    // borrows cross method boundaries.
    let Runtime {
        memory,
        registry,
        hle_heap_ptr,
        hle_next_id,
        gcm_state,
        ..
    } = runtime;
    let mut ctx = RuntimeHleAdapter {
        memory,
        registry,
        heap_ptr: hle_heap_ptr,
        next_id: hle_next_id,
        source,
    };
    init_body(&mut ctx, args, gcm_state);
}

fn get_configuration_with_runtime(runtime: &mut Runtime, source: UnitId, args: &[u64; 9]) {
    let Runtime {
        memory,
        registry,
        hle_heap_ptr,
        hle_next_id,
        gcm_state,
        ..
    } = runtime;
    let mut ctx = RuntimeHleAdapter {
        memory,
        registry,
        heap_ptr: hle_heap_ptr,
        next_id: hle_next_id,
        source,
    };
    get_configuration(&mut ctx, args, gcm_state);
}

pub(crate) fn get_tiled_pitch_size(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let size = args[1] as u32;
    let result = tiled_pitch_lookup(size);
    ctx.set_return(result as u64);
}

pub(crate) fn init_body(ctx: &mut dyn HleContext, args: &[u64; 9], state: &mut GcmState) {
    let context_pp = args[1] as u32;
    let _cmd_size = args[2] as u32;
    let io_size = args[3] as u32;
    let io_address = args[4] as u32;

    state.io_address = io_address;
    state.io_size = io_size;
    state.local_size = 0x0f90_0000;

    let cb_base = ctx.heap_alloc(16, 16);
    let cb_opd = cb_base;
    let cb_body = cb_base + 8;
    let mut cb_buf = [0u8; 16];
    cb_buf[0..4].copy_from_slice(&cb_body.to_be_bytes());
    cb_buf[8..12].copy_from_slice(&0x3860_0000u32.to_be_bytes());
    cb_buf[12..16].copy_from_slice(&0x4E80_0020u32.to_be_bytes());
    ctx.write_guest(cb_base as u64, &cb_buf);

    let ctx_addr = ctx.heap_alloc(16, 16);
    state.context_addr = ctx_addr;

    let begin = io_address + 0x1000;
    let end = io_address + io_size - 4;
    let mut ctx_buf = [0u8; 16];
    ctx_buf[0..4].copy_from_slice(&begin.to_be_bytes());
    ctx_buf[4..8].copy_from_slice(&end.to_be_bytes());
    ctx_buf[8..12].copy_from_slice(&begin.to_be_bytes());
    ctx_buf[12..16].copy_from_slice(&cb_opd.to_be_bytes());
    ctx.write_guest(ctx_addr as u64, &ctx_buf);

    ctx.write_guest(context_pp as u64, &ctx_addr.to_be_bytes());

    let ctrl_addr = if state.rsx_checkpoint {
        0xC000_0040u32
    } else {
        ctx.heap_alloc(12, 16)
    };
    state.control_addr = ctrl_addr;

    let label_addr = ctx.heap_alloc(4096, 16);
    state.label_addr = label_addr;
    let label_fill = vec![0xFFu8; 4096];
    ctx.write_guest(label_addr as u64, &label_fill);

    ctx.set_return(0);
}

pub(crate) fn get_configuration(ctx: &mut dyn HleContext, args: &[u64; 9], state: &GcmState) {
    let config_ptr = args[1] as u32;
    let mut buf = [0u8; 24];
    buf[0..4].copy_from_slice(&0xC000_0000u32.to_be_bytes());
    buf[4..8].copy_from_slice(&state.io_address.to_be_bytes());
    buf[8..12].copy_from_slice(&state.local_size.to_be_bytes());
    buf[12..16].copy_from_slice(&state.io_size.to_be_bytes());
    buf[16..20].copy_from_slice(&650_000_000u32.to_be_bytes());
    buf[20..24].copy_from_slice(&500_000_000u32.to_be_bytes());
    ctx.write_guest(config_ptr as u64, &buf);
    ctx.set_return(0);
}

pub fn tiled_pitch_lookup(size: u32) -> u32 {
    TILED_PITCHES
        .windows(2)
        .find(|w| w[0] < size && size <= w[1])
        .map(|w| w[1])
        .unwrap_or(0)
}
