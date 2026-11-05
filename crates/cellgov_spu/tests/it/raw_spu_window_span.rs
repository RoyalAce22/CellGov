//! A write effect that runs from one Raw SPU's window into the next
//! one's: each window takes its own part.
//!
//! The scripted PPU emits the effect directly. A PPU store cannot make
//! one: the windows are caching-inhibited, so such a store takes the
//! alignment interrupt (`spu_raw_window_span`).

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_ps3_abi::lv2::spu::raw_window;
use cellgov_ps3_abi::lv2::syscall::{RAW_SPU_CREATE, SPU_INITIALIZE};
use cellgov_spu::SpuExecutionUnit;

use crate::raw_spu_window::{run_ops, runtime, Op, OUT_PTR, WINDOW};

#[test]
fn a_write_effect_across_two_windows_lands_its_tail_in_the_next_raw_spus_local_store() {
    let value = 0xA1B2_C3D4u32;
    let ops = vec![
        Op::Call([SPU_INITIALIZE, 6, 2, 0]),
        Op::Call([RAW_SPU_CREATE, OUT_PTR, 0, 0]),
        Op::Call([RAW_SPU_CREATE, OUT_PTR + 8, 0, 0]),
        // The last two bytes of window 0's stride and the first two of window 1.
        Op::Store(WINDOW + raw_window::STRIDE - 2, value),
    ];
    let (mut rt, ppu) = runtime(ops);
    run_ops(&mut rt, ppu, 4);
    let raw = rt.lv2_host().raw_spu_unit(1).unwrap();
    let spu = rt.registry().get(raw).unwrap().as_any();
    let ls = &spu.downcast_ref::<SpuExecutionUnit>().unwrap().state().ls;
    assert_eq!(
        ls[..2],
        value.to_be_bytes()[2..],
        "Raw SPU 1 takes the part of the store in its window"
    );
    // The head lands past Raw SPU 0's local store and on no whole
    // register, so its window refuses it by name.
    assert_eq!(
        rt.lv2_host()
            .invariant_break_site_count("runtime.raw_spu_window_access_refused"),
        1,
        "Raw SPU 0 refuses the part of the store in its window"
    );
}
