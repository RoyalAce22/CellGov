//! DmaLatencyModel contract exercised through a linear model: rounding, determinism, monotonicity.

use super::*;
use crate::request::DmaDirection;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};

struct LinearLatency {
    bytes_per_tick: u64,
}

impl DmaLatencyModel for LinearLatency {
    fn completion_time(&self, req: &DmaRequest, now: GuestTicks) -> GuestTicks {
        let len = req.length();
        let ticks = len.div_ceil(self.bytes_per_tick);
        now.checked_add(GuestTicks::new(ticks))
            .expect("completion time within u64 range")
    }
}

fn req(length: u64) -> DmaRequest {
    DmaRequest::new(
        DmaDirection::Put,
        ByteRange::new(GuestAddr::new(0x1000), length).unwrap(),
        ByteRange::new(GuestAddr::new(0x9000), length).unwrap(),
        UnitId::new(0),
    )
    .unwrap()
}

#[test]
fn linear_model_basic() {
    let model = LinearLatency { bytes_per_tick: 16 };
    let r = req(64);
    let t = model.completion_time(&r, GuestTicks::new(100));
    assert_eq!(t, GuestTicks::new(104));
}

#[test]
fn linear_model_round_up() {
    let model = LinearLatency { bytes_per_tick: 16 };
    let r = req(17);
    let t = model.completion_time(&r, GuestTicks::new(0));
    assert_eq!(t, GuestTicks::new(2));
}

#[test]
fn linear_model_zero_length_completes_at_now() {
    let model = LinearLatency { bytes_per_tick: 16 };
    let r = req(0);
    let t = model.completion_time(&r, GuestTicks::new(50));
    assert_eq!(t, GuestTicks::new(50));
}

#[test]
fn linear_model_is_deterministic() {
    let model = LinearLatency { bytes_per_tick: 8 };
    let r = req(40);
    let now = GuestTicks::new(1000);
    let a = model.completion_time(&r, now);
    let b = model.completion_time(&r, now);
    assert_eq!(a, b);
}

#[test]
fn linear_model_is_monotone_in_now() {
    let model = LinearLatency { bytes_per_tick: 8 };
    let r = req(40);
    let earlier = model.completion_time(&r, GuestTicks::new(100));
    let later = model.completion_time(&r, GuestTicks::new(200));
    assert!(earlier < later);
}
