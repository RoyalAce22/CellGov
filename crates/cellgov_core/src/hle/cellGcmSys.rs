//! cellGcmSys HLE implementations.
//!
//! RSX graphics library (RPCS3 `Modules/cellGcmSys.cpp`). Real
//! behavior is gated behind the runtime's RSX-checkpoint flag; if
//! the flag is off, [`dispatch`] returns `None` so the router
//! falls through to the default CELL_OK fallback.
//!
//! ## Failure policy
//!
//! Fidelity to real PS3 beats defensive validation. Arithmetic on
//! guest-controlled values (args, io_size, label indices) uses
//! `wrapping_*` because the real PPU hardware wraps, and a bounds
//! check here would diverge from a comparison run against RPCS3 or
//! a PS3 trace. Arithmetic on values we control (heap pointers we
//! just allocated) uses plain `+`; a wrap there is an oracle bug,
//! not hardware behavior.
//!
//! `debug_assert!` marks invariants that a well-behaved guest
//! cannot violate on real hardware (re-init, queries before init).
//! Triggering one in tests means the harness, not the guest, is
//! driving the HLE in an impossible order. `.expect(...)` stays
//! reserved for oracle-state corruption (heap exhaustion, commit
//! failure).
//!
//! ## Comparison-run note
//!
//! The RSX-checkpoint flag itself is a divergence source: the same
//! NID produces different behavior depending on a flag that real
//! PS3 and RPCS3 do not have. Cross-runner comparison runs must
//! configure the flag consistently on both sides or the diff will
//! flag the gate itself rather than anything substantive.

use cellgov_event::UnitId;

use crate::hle::context::{HleContext, RuntimeHleAdapter};
use crate::runtime::Runtime;

pub(crate) const NID_CELLGCM_GET_TILED_PITCH_SIZE: u32 = 0x055bd74d;
pub(crate) const NID_CELLGCM_INIT_BODY: u32 = 0x15bae46b;
pub(crate) const NID_CELLGCM_GET_CONFIGURATION: u32 = 0xe315a0b2;
pub(crate) const NID_CELLGCM_GET_CONTROL_REGISTER: u32 = 0xa547adde;
pub(crate) const NID_CELLGCM_GET_LABEL_ADDRESS: u32 = 0xf80196c1;

/// Every NID this module claims. See [`crate::hle::sys_prx_for_user::OWNED_NIDS`]
/// for the disjointness contract and the drift-canary test pattern
/// reproduced in this module's own `canary_tests` block. These NIDs
/// are owned regardless of the `rsx_checkpoint` flag -- disabling at
/// the dispatch boundary affects *whether* cellGcmSys runs real
/// handlers, not which NIDs it owns.
#[cfg(test)]
pub(crate) const OWNED_NIDS: &[u32] = &[
    NID_CELLGCM_GET_TILED_PITCH_SIZE,
    NID_CELLGCM_INIT_BODY,
    NID_CELLGCM_GET_CONFIGURATION,
    NID_CELLGCM_GET_CONTROL_REGISTER,
    NID_CELLGCM_GET_LABEL_ADDRESS,
];

/// Per-module state for cellGcmSys. Owned by the HLE dispatch layer,
/// not by Runtime. Zero-valued `context_addr`, `control_addr`, and
/// `label_addr` indicate "not yet initialized" -- no separate flag
/// is needed because all three become non-zero after `init_body`
/// runs. Once set, they remain non-zero across re-inits (each
/// subsequent `init_body` call overwrites them with a fresh non-zero
/// value); see `init_body` for re-init semantics and the documented
/// heap leak that attends it.
///
/// ## Structural witness
///
/// The "non-zero == initialized" contract depends on two external
/// facts, each enforced structurally rather than by convention:
///
/// 1. The RSX-checkpoint MMIO sentinel `0xC000_0040` is non-zero.
///    A canary test pins this and fires if the constant ever
///    changes; see `crate::hle::tests::gcm_init_body_control_addr_is_nonzero_in_both_modes`.
/// 2. The HLE bump allocator cannot hand out address 0. The
///    assertion lives in `RuntimeHleAdapter::heap_alloc`
///    (`debug_assert_ne!(aligned, 0, ...)`), backed by the
///    `set_hle_heap_base` precondition that forbids `base = 0`.
///
/// Both facts are required: (1) covers the checkpoint branch of
/// `init_body`, (2) covers the fallback branch. Together they make
/// the zero-valued witness a genuine structural invariant rather
/// than a coincidence.
#[derive(Debug, Default)]
pub(crate) struct GcmState {
    pub(crate) context_addr: u32,
    /// Non-zero post-init. Under `rsx_checkpoint` this is the
    /// fixed MMIO sentinel `0xC000_0040`; otherwise it is a
    /// non-zero heap allocation (see the `heap_alloc` structural
    /// guarantee documented on the struct). The
    /// `debug_assert_ne!(..., 0)` witness in the
    /// GET_CONTROL_REGISTER dispatch path is a genuine contract
    /// rather than a convention -- see the struct doc.
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
    if !runtime.hle.gcm.rsx_checkpoint {
        return None;
    }
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
            // Real cellGcm does not bounds-check the index: it is
            // just pointer math. An out-of-range index returns a
            // pointer past the label region; a misbehaving guest
            // then corrupts whatever the HLE heap placed next, the
            // same as on real hardware. Wrapping arithmetic makes
            // the overflow path explicit and release/debug agree.
            let offset = 0x10u32.wrapping_mul(index);
            let addr = state.label_addr.wrapping_add(offset) as u64;
            adapter(runtime, source, nid).set_return(addr);
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
    // Split-borrow: the gcm substate and the ctx fields both sit
    // under Runtime.hle; construct the adapter first, then pass
    // &mut gcm to the handler. Both borrows cross method boundaries
    // so destructure to disjoint borrows here. `heap_base` is a
    // Copy snapshot so it does not compete for the borrow of
    // `runtime.hle`.
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
        nid: NID_CELLGCM_INIT_BODY,
        mutated: false,
        handlers_without_mutation,
    };
    init_body(&mut ctx, args, gcm);
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
    // cmd_size is the guest-declared command-queue capacity. We do
    // not model a separate command queue: the guest's io region
    // (`io_address..io_address + io_size`) *is* the command queue
    // for our purposes. RPCS3's HLE path also ignores this value.
    // Do not "fix" the unused prefix -- the argument is intentional
    // ABI padding.
    let _cmd_size_unused_hle = args[2] as u32;
    let io_size = args[3] as u32;
    let io_address = args[4] as u32;

    // Re-init is legal. RPCS3's `_cellGcmInitBody` (see
    // `tools/rpcs3-src/rpcs3/Emu/Cell/Modules/cellGcmSys.cpp`,
    // ~line 397) explicitly resets its state fields at entry
    // and proceeds; there is no ALREADY_INITIALIZED error path.
    // Some shipped titles re-init GCM after video-mode changes
    // or XMB return, so asserting against a second call would
    // fire on legitimate guest behavior.
    //
    // Known leak: our bump allocator cannot release the first
    // init's context / control / label / command-buffer
    // allocations when a second init reallocates them. RPCS3
    // leaks the same regions for the same reason (bump-style
    // falloc). The leak is bounded (each init reserves ~4 KB
    // plus the label region) and does not affect determinism.
    // Downstream fields (`state.context_addr` etc.) are
    // overwritten by the new heap_alloc calls below, matching
    // RPCS3's reset-and-continue semantics.

    // Guest-controlled pointer math on real PS3 wraps. Make that
    // explicit so release and debug agree and no release-mode
    // silent overflow ever sneaks in.
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
    let label_fill = vec![0xFFu8; 4096];
    ctx.write_guest(label_addr as u64, &label_fill)
        .expect("cellGcmInitBody: label-region fill failed");

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
    // Real PS3 RSX clocks -- not placeholders, not configurable.
    buf[16..20].copy_from_slice(&650_000_000u32.to_be_bytes());
    buf[20..24].copy_from_slice(&500_000_000u32.to_be_bytes());
    ctx.write_guest(config_ptr as u64, &buf)
        .expect("cellGcmGetConfiguration: write to config out-ptr failed");
    ctx.set_return(0);
}

/// Look up the tiled-surface pitch corresponding to a requested
/// size.
///
/// Returns `0` for `size == 0` (no valid pitch; `0x000` is the
/// lower bound of the first window, not a valid pitch itself) and
/// for `size > 0x10000` (above the largest supported pitch). Every
/// other value returns the smallest pitch in [`TILED_PITCHES`] that
/// is >= `size`. Both zero-returning cases match RPCS3 bit-for-bit
/// (see `tools/rpcs3-src/rpcs3/Emu/Cell/Modules/cellGcmSys.cpp`
/// `cellGcmGetTiledPitchSize`, lines 363-373: the `for` loop falls
/// through to `return 0;` on no-match for both the below-window
/// and above-window cases). Callers treat `0` as "no valid tiled
/// pitch for this size" and fall back to the guest's linear path.
///
/// The `0x200` boundary is inclusive on the upper side: `size ==
/// 0x200` matches the first window `(0x000, 0x200]` and returns
/// `0x200`. `size == 0x001` also matches the same window and
/// returns `0x200`. `size == 0x201` falls into the next window
/// `(0x200, 0x300]` and returns `0x300`. `size == 0x000` does
/// not match (no window has `w[0] < 0`) and returns the sentinel
/// `0`. Boundary behavior is covered by the
/// `tiled_pitch_*` tests in `crate::hle`.
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

    /// Build a runtime with the rsx_checkpoint gate open and gcm
    /// state pre-seeded as if `init_body` had already run. The
    /// canary only checks that `dispatch` claims the NID; the
    /// GET_CONTROL_REGISTER / GET_LABEL_ADDRESS / GET_CONFIGURATION
    /// handlers trip `debug_assert_ne!(_, 0)` witnesses against the
    /// gcm substate, so we seed non-zero values here.
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

    /// With `rsx_checkpoint` disabled, dispatch short-circuits to
    /// `None` for every NID -- even ones the module owns. Pinning
    /// this pre-gate here means a refactor that reorders the gate
    /// vs. the match cannot silently start running gcm handlers on
    /// non-RSX boots.
    #[test]
    fn rsx_checkpoint_gate_blocks_all_owned_nids() {
        for &nid in OWNED_NIDS {
            let (mut rt, unit_id) = canary_runtime();
            rt.set_gcm_rsx_checkpoint(false);
            let args: [u64; 9] = [0; 9];
            let result = dispatch(&mut rt, unit_id, nid, &args);
            assert_eq!(
                result, None,
                "gcm::dispatch claimed NID {nid:#010x} with rsx_checkpoint off"
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
