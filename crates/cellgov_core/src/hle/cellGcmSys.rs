//! cellGcmSys HLE implementations.
//!
//! ## Failure policy
//!
//! Arithmetic on guest-controlled values uses `wrapping_*` to match
//! real PPU hardware; arithmetic on oracle-owned values (fresh heap
//! pointers) uses plain `+` so a wrap is a loud oracle bug.
//! `debug_assert!` marks invariants a well-behaved guest cannot
//! violate on real hardware; `.expect(...)` is reserved for
//! oracle-state corruption (heap exhaustion, commit failure).
//!
//! ## Comparison-run note
//!
//! The RSX-checkpoint flag is a divergence source: the same NID
//! behaves differently based on a flag real PS3 and RPCS3 do not
//! have. Cross-runner comparison runs must configure it
//! consistently on both sides.

use cellgov_event::UnitId;

use crate::hle::context::{HleContext, RuntimeHleAdapter};
use crate::runtime::Runtime;

pub(crate) const NID_CELLGCM_GET_TILED_PITCH_SIZE: u32 = 0x055bd74d;
pub(crate) const NID_CELLGCM_INIT_BODY: u32 = 0x15bae46b;
pub(crate) const NID_CELLGCM_GET_CONFIGURATION: u32 = 0xe315a0b2;
pub(crate) const NID_CELLGCM_GET_CONTROL_REGISTER: u32 = 0xa547adde;
pub(crate) const NID_CELLGCM_GET_LABEL_ADDRESS: u32 = 0xf80196c1;
/// `cellGcmSetFlipHandler`. The oracle records the callback address
/// into [`crate::rsx::flip::RsxFlipState::handler`] but does not
/// dispatch PPU execution into it.
pub(crate) const NID_CELLGCM_SET_FLIP_HANDLER: u32 = 0xa41ef7e8;

/// Every NID this module claims. See
/// [`crate::hle::sys_prx_for_user::OWNED_NIDS`] for the disjointness
/// contract.
#[cfg(test)]
pub(crate) const OWNED_NIDS: &[u32] = &[
    NID_CELLGCM_GET_TILED_PITCH_SIZE,
    NID_CELLGCM_INIT_BODY,
    NID_CELLGCM_GET_CONFIGURATION,
    NID_CELLGCM_GET_CONTROL_REGISTER,
    NID_CELLGCM_GET_LABEL_ADDRESS,
    NID_CELLGCM_SET_FLIP_HANDLER,
];

/// Per-module state for cellGcmSys.
///
/// Zero-valued `context_addr`, `control_addr`, and `label_addr` mean
/// "not yet initialized"; all three become non-zero after `init_body`
/// runs. The witness holds because (1) the RSX-checkpoint MMIO
/// sentinel `0xC000_0040` is non-zero and (2) the HLE bump allocator
/// refuses to hand out address 0 (see `RuntimeHleAdapter::heap_alloc`
/// and the `set_hle_heap_base` precondition).
#[derive(Debug, Default)]
pub(crate) struct GcmState {
    pub(crate) context_addr: u32,
    /// Non-zero post-init. `0xC000_0040` under `rsx_checkpoint`,
    /// otherwise a heap allocation. The GET_CONTROL_REGISTER
    /// dispatch `debug_assert_ne!(..., 0)` relies on this.
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

/// Dispatch entry point; returns `None` if the NID is not owned here.
pub(crate) fn dispatch(
    runtime: &mut Runtime,
    source: UnitId,
    nid: u32,
    args: &[u64; 9],
) -> Option<()> {
    match nid {
        NID_CELLGCM_GET_TILED_PITCH_SIZE => {
            get_tiled_pitch_size(&mut adapter(runtime, source, nid), args);
        }
        NID_CELLGCM_INIT_BODY => {
            init_body_with_runtime(runtime, source, args);
        }
        NID_CELLGCM_GET_CONFIGURATION => {
            get_configuration_with_runtime(runtime, source, args);
        }
        NID_CELLGCM_GET_CONTROL_REGISTER => {
            let state = &runtime.hle.gcm;
            debug_assert_ne!(
                state.control_addr, 0,
                "cellGcmGetControlRegister called before cellGcmInitBody (control_addr is still 0)"
            );
            let ctrl = state.control_addr as u64;
            adapter(runtime, source, nid).set_return(ctrl);
        }
        NID_CELLGCM_GET_LABEL_ADDRESS => {
            let index = args[1] as u32;
            let state = &runtime.hle.gcm;
            debug_assert_ne!(
                state.label_addr, 0,
                "cellGcmGetLabelAddress called before cellGcmInitBody (label_addr is still 0)"
            );
            // Real cellGcm does not bounds-check the index; wrapping
            // matches hardware and keeps debug/release in agreement.
            let offset = 0x10u32.wrapping_mul(index);
            let addr = state.label_addr.wrapping_add(offset) as u64;
            adapter(runtime, source, nid).set_return(addr);
        }
        NID_CELLGCM_SET_FLIP_HANDLER => {
            // Zero clears. Record in `RsxFlipState.handler` and
            // mirror to `SysRsxContext.flip_handler_addr` so sys_rsx
            // stays the single source of truth.
            let handler_addr = args[1] as u32;
            runtime.rsx_flip_mut().set_handler(handler_addr);
            if runtime.lv2_host().sys_rsx_context().allocated {
                runtime.dispatch_lv2_request(
                    cellgov_lv2::Lv2Request::SysRsxContextAttribute {
                        context_id: runtime.lv2_host().sys_rsx_context().context_id,
                        package_id: cellgov_lv2::host::PACKAGE_CELLGOV_SET_FLIP_HANDLER,
                        a3: handler_addr as u64,
                        a4: 0,
                        a5: 0,
                        a6: 0,
                    },
                    source,
                );
            }
            adapter(runtime, source, nid).set_return(0);
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

fn init_body_with_runtime(runtime: &mut Runtime, source: UnitId, args: &[u64; 9]) {
    // Split-borrow: gcm substate and ctx fields both sit under
    // Runtime.hle; destructure to disjoint borrows here.
    let rsx_checkpoint = runtime.hle.gcm.rsx_checkpoint;
    let heap_base = runtime.hle.heap_base;
    {
        let Runtime {
            memory,
            registry,
            hle:
                crate::hle::HleState {
                    heap_ptr,
                    heap_watermark,
                    heap_warning_mask,
                    next_id,
                    gcm,
                    handlers_without_mutation,
                    ..
                },
            ..
        } = runtime;
        let mut ctx = RuntimeHleAdapter {
            memory,
            registry,
            heap_base,
            heap_ptr,
            heap_watermark,
            heap_warning_mask,
            next_id,
            source,
            nid: NID_CELLGCM_INIT_BODY,
            mutated: false,
            handlers_without_mutation,
        };
        init_body(&mut ctx, args, gcm);
    }
    if !rsx_checkpoint {
        forward_init_to_sys_rsx(runtime, source);
    }
}

/// Non-checkpoint half of `cellGcmInitBody`: fire
/// sys_rsx_memory_allocate + sys_rsx_context_allocate, read the
/// canonical addresses back from Lv2Host's SysRsxContext.
fn forward_init_to_sys_rsx(runtime: &mut Runtime, source: UnitId) {
    use cellgov_lv2::Lv2Request;
    // 48 bytes of scratch for six out-pointers totaling 36 bytes,
    // rounded for 16-byte alignment.
    let scratch = runtime
        .hle
        .heap_ptr
        .checked_add(16 - (runtime.hle.heap_ptr & 0xF))
        .expect("cellGcmInitBody: heap cursor overflow aligning sys_rsx scratch");
    runtime.hle.heap_ptr = scratch + 48;
    if runtime.hle.heap_ptr > runtime.hle.heap_watermark {
        runtime.hle.heap_watermark = runtime.hle.heap_ptr;
    }

    runtime.dispatch_lv2_request(
        Lv2Request::SysRsxMemoryAllocate {
            mem_handle_ptr: scratch,
            mem_addr_ptr: scratch + 4,
            size: 0x0030_0000,
            flags: 0,
            a5: 0,
            a6: 0,
            a7: 0,
        },
        source,
    );
    let mem_handle_bytes = runtime
        .memory()
        .as_bytes()
        .get(scratch as usize..scratch as usize + 4)
        .expect("cellGcmInitBody: scratch ptr out of bounds");
    let mem_ctx = u64::from(u32::from_be_bytes([
        mem_handle_bytes[0],
        mem_handle_bytes[1],
        mem_handle_bytes[2],
        mem_handle_bytes[3],
    ]));

    runtime.dispatch_lv2_request(
        Lv2Request::SysRsxContextAllocate {
            context_id_ptr: scratch + 12,
            lpar_dma_control_ptr: scratch + 16,
            lpar_driver_info_ptr: scratch + 24,
            lpar_reports_ptr: scratch + 32,
            mem_ctx,
            system_mode: 0,
        },
        source,
    );

    let rsx = *runtime.lv2_host().sys_rsx_context();
    // Skip past the 0x40-byte reserved prefix inside RsxDmaControl;
    // cellGcmGetControlRegister returns the put/get/ref window base.
    runtime.hle.gcm.control_addr = rsx.dma_control_addr + 0x40;
    runtime.hle.gcm.label_addr = rsx.reports_addr;
}

fn get_configuration_with_runtime(runtime: &mut Runtime, source: UnitId, args: &[u64; 9]) {
    let heap_base = runtime.hle.heap_base;
    let Runtime {
        memory,
        registry,
        hle:
            crate::hle::HleState {
                heap_ptr,
                heap_watermark,
                heap_warning_mask,
                next_id,
                gcm,
                handlers_without_mutation,
                ..
            },
        ..
    } = runtime;
    let mut ctx = RuntimeHleAdapter {
        memory,
        registry,
        heap_base,
        heap_ptr,
        heap_watermark,
        heap_warning_mask,
        next_id,
        source,
        nid: NID_CELLGCM_GET_CONFIGURATION,
        mutated: false,
        handlers_without_mutation,
    };
    get_configuration(&mut ctx, args, gcm);
}

pub(crate) fn get_tiled_pitch_size(ctx: &mut dyn HleContext, args: &[u64; 9]) {
    let size = args[1] as u32;
    let result = tiled_pitch_lookup(size);
    ctx.set_return(result as u64);
}

pub(crate) fn init_body(ctx: &mut dyn HleContext, args: &[u64; 9], state: &mut GcmState) {
    let context_pp = args[1] as u32;
    // The guest's io region IS the command queue; cmd_size is ABI
    // padding (RPCS3 ignores it too).
    let _cmd_size_unused_hle = args[2] as u32;
    let io_size = args[3] as u32;
    let io_address = args[4] as u32;

    // Re-init is legal: Sony's `_cellGcmInitBody` resets and
    // proceeds. Each re-init leaks the prior context / control /
    // label / command-buffer allocations because the bump allocator
    // cannot release them (RPCS3 has the same bounded leak).

    // Guest-controlled pointer math wraps on real PS3.
    let begin = io_address.wrapping_add(0x1000);
    let end = io_address.wrapping_add(io_size).wrapping_sub(4);

    state.io_address = io_address;
    state.io_size = io_size;
    state.local_size = 0x0f90_0000;

    let cb_base = ctx
        .heap_alloc(16, 16)
        .expect("cellGcmInitBody: HLE heap exhausted (command buffer OPD)");
    let cb_opd = cb_base;
    let cb_body = cb_base + 8;
    let mut cb_buf = [0u8; 16];
    cb_buf[0..4].copy_from_slice(&cb_body.to_be_bytes());
    cb_buf[8..12].copy_from_slice(&0x3860_0000u32.to_be_bytes());
    cb_buf[12..16].copy_from_slice(&0x4E80_0020u32.to_be_bytes());
    ctx.write_guest(cb_base as u64, &cb_buf)
        .expect("cellGcmInitBody: write to command buffer failed");

    let ctx_addr = ctx
        .heap_alloc(16, 16)
        .expect("cellGcmInitBody: HLE heap exhausted (context struct)");
    state.context_addr = ctx_addr;

    let mut ctx_buf = [0u8; 16];
    ctx_buf[0..4].copy_from_slice(&begin.to_be_bytes());
    ctx_buf[4..8].copy_from_slice(&end.to_be_bytes());
    ctx_buf[8..12].copy_from_slice(&begin.to_be_bytes());
    ctx_buf[12..16].copy_from_slice(&cb_opd.to_be_bytes());
    ctx.write_guest(ctx_addr as u64, &ctx_buf)
        .expect("cellGcmInitBody: write to context struct failed");

    ctx.write_guest(context_pp as u64, &ctx_addr.to_be_bytes())
        .expect("cellGcmInitBody: write to context double-pointer failed");

    let ctrl_addr = if state.rsx_checkpoint {
        0xC000_0040u32
    } else {
        ctx.heap_alloc(12, 16)
            .expect("cellGcmInitBody: HLE heap exhausted (control register)")
    };
    state.control_addr = ctrl_addr;

    let label_addr = ctx
        .heap_alloc(4096, 16)
        .expect("cellGcmInitBody: HLE heap exhausted (label region)");
    state.label_addr = label_addr;
    // Labels start at 0 (zero-init from GuestMemory). FIFO advance
    // is the sole source of non-zero label values; a pre-fill would
    // mask divergences.

    ctx.set_return(0);
}

pub(crate) fn get_configuration(ctx: &mut dyn HleContext, args: &[u64; 9], state: &GcmState) {
    debug_assert_ne!(
        state.context_addr, 0,
        "cellGcmGetConfiguration called before cellGcmInitBody (context_addr is still 0)"
    );
    let config_ptr = args[1] as u32;
    let mut buf = [0u8; 24];
    buf[0..4].copy_from_slice(&0xC000_0000u32.to_be_bytes());
    buf[4..8].copy_from_slice(&state.io_address.to_be_bytes());
    buf[8..12].copy_from_slice(&state.local_size.to_be_bytes());
    buf[12..16].copy_from_slice(&state.io_size.to_be_bytes());
    // Real PS3 RSX clocks.
    buf[16..20].copy_from_slice(&650_000_000u32.to_be_bytes());
    buf[20..24].copy_from_slice(&500_000_000u32.to_be_bytes());
    ctx.write_guest(config_ptr as u64, &buf)
        .expect("cellGcmGetConfiguration: write to config out-ptr failed");
    ctx.set_return(0);
}

/// Smallest tiled pitch in [`TILED_PITCHES`] >= `size`.
///
/// Returns `0` for `size == 0` and for `size > 0x10000`; callers
/// treat `0` as "no valid tiled pitch" and fall back to the linear
/// path. Windows are `(low, high]` inclusive on the upper side.
pub fn tiled_pitch_lookup(size: u32) -> u32 {
    TILED_PITCHES
        .windows(2)
        .find(|w| w[0] < size && size <= w[1])
        .map(|w| w[1])
        .unwrap_or(0)
}

#[cfg(test)]
mod canary_tests {
    use super::{dispatch, OWNED_NIDS};
    use crate::runtime::Runtime;
    use cellgov_event::UnitId;
    use cellgov_exec::{FakeIsaUnit, FakeOp};
    use cellgov_mem::GuestMemory;
    use cellgov_time::Budget;

    /// Runtime with rsx_checkpoint on and gcm state pre-seeded as if
    /// `init_body` had run. Non-zero seeds satisfy the
    /// `debug_assert_ne!(_, 0)` witnesses in the GET_* handlers.
    fn canary_runtime() -> (Runtime, UnitId) {
        let mut rt = Runtime::new(GuestMemory::new(0x20_0000), Budget::new(1), 100);
        let unit_id = UnitId::new(0);
        rt.registry_mut()
            .register_with(|id| FakeIsaUnit::new(id, vec![FakeOp::End]));
        rt.set_hle_heap_base(0x10_0000);
        rt.set_gcm_rsx_checkpoint(true);
        rt.hle.gcm.context_addr = 0x11_0000;
        rt.hle.gcm.control_addr = 0xC000_0040;
        rt.hle.gcm.label_addr = 0x12_0000;
        (rt, unit_id)
    }

    /// Drift canary for [`OWNED_NIDS`] vs the [`dispatch`] match arms.
    /// Mirror of `sys::canary_tests::owned_nids_all_claimed_by_dispatch`;
    /// see that test's rustdoc for the full rationale.
    #[test]
    fn owned_nids_all_claimed_by_dispatch() {
        for &nid in OWNED_NIDS {
            let (mut rt, unit_id) = canary_runtime();
            let args: [u64; 9] = [0; 9];
            let result = dispatch(&mut rt, unit_id, nid, &args);
            assert_eq!(
                result,
                Some(()),
                "gcm::dispatch returned None for NID {nid:#010x} listed in OWNED_NIDS \
                 -- the match arm was likely removed without trimming the list"
            );
        }
    }

    /// Negative companion to the coverage canary: sys-owned NIDs
    /// and a synthetic never-registered NID must return `None` from
    /// gcm's dispatch.
    #[test]
    fn unowned_nids_are_rejected_by_dispatch() {
        let probes: &[u32] = &[
            crate::hle::sys_prx_for_user::NID_SYS_MALLOC,
            crate::hle::sys_prx_for_user::NID_SYS_PPU_THREAD_GET_ID,
            0xDEAD_BEEF,
        ];
        for &nid in probes {
            let (mut rt, unit_id) = canary_runtime();
            let args: [u64; 9] = [0; 9];
            let result = dispatch(&mut rt, unit_id, nid, &args);
            assert_eq!(
                result, None,
                "gcm::dispatch claimed NID {nid:#010x} that is not in its OWNED_NIDS"
            );
        }
    }
}
