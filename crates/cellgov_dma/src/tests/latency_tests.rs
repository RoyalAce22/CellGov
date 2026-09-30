//! DmaLatencyModel contract exercised through a linear model: rounding, determinism, monotonicity.

use super::*;
use crate::request::DmaDirection;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};

struct LinearLatency {
    bytes_per_tick: u64,
}

impl DmaLatencyModel for LinearLatency {
    fn completion_time(&self, req: &DmaRequest, now: GuestTicks, _queued: &DmaQueue) -> GuestTicks {
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
    let t = model.completion_time(&r, GuestTicks::new(100), &DmaQueue::new());
    assert_eq!(t, GuestTicks::new(104));
}

#[test]
fn linear_model_round_up() {
    let model = LinearLatency { bytes_per_tick: 16 };
    let r = req(17);
    let t = model.completion_time(&r, GuestTicks::new(0), &DmaQueue::new());
    assert_eq!(t, GuestTicks::new(2));
}

#[test]
fn linear_model_zero_length_completes_at_now() {
    let model = LinearLatency { bytes_per_tick: 16 };
    let r = req(0);
    let t = model.completion_time(&r, GuestTicks::new(50), &DmaQueue::new());
    assert_eq!(t, GuestTicks::new(50));
}

#[test]
fn linear_model_is_deterministic() {
    let model = LinearLatency { bytes_per_tick: 8 };
    let r = req(40);
    let now = GuestTicks::new(1000);
    let a = model.completion_time(&r, now, &DmaQueue::new());
    let b = model.completion_time(&r, now, &DmaQueue::new());
    assert_eq!(a, b);
}

#[test]
fn linear_model_is_monotone_in_now() {
    let model = LinearLatency { bytes_per_tick: 8 };
    let r = req(40);
    let earlier = model.completion_time(&r, GuestTicks::new(100), &DmaQueue::new());
    let later = model.completion_time(&r, GuestTicks::new(200), &DmaQueue::new());
    assert!(earlier < later);
}

/// A model that charges each request for the transfers queued ahead of it.
struct QueuedLatency;

impl DmaLatencyModel for QueuedLatency {
    fn completion_time(&self, _req: &DmaRequest, now: GuestTicks, queued: &DmaQueue) -> GuestTicks {
        now.checked_add(GuestTicks::new(10 * (queued.len() as u64 + 1)))
            .expect("completion time within u64 range")
    }
}

#[test]
fn a_model_sees_the_commands_queued_ahead() {
    use crate::completion::DmaCompletion;
    let model = QueuedLatency;
    let mut queue = DmaQueue::new();
    let r = req(16);
    let first = model.completion_time(&r, GuestTicks::new(0), &queue);
    queue.enqueue(DmaCompletion::new(r, first), None);
    let second = model.completion_time(&r, GuestTicks::new(0), &queue);
    assert_eq!((first, second), (GuestTicks::new(10), GuestTicks::new(20)));
}
