//! A put from local store, queued as its bus requests.

use super::queue_transfer;
use cellgov_dma::{DmaDirection, DmaLatencyModel, DmaQueue, DmaRequest, FixedLatency, MfcOrdering};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::hw::spu::MfcTagId;
use cellgov_time::GuestTicks;

fn put(len: u64) -> DmaRequest {
    let range = |at| ByteRange::new(GuestAddr::new(at), len).expect("range fits");
    DmaRequest::new(
        DmaDirection::Put,
        range(0x1000),
        range(0x10_0000),
        UnitId::new(1),
    )
    .expect("equal lengths")
    .with_local_store_source()
}

/// Every queued transfer's completion time, its length, and whether it
/// holds a slot, in queue order.
fn queued(queue: &DmaQueue) -> Vec<(u64, u64, bool)> {
    let mut entries: Vec<_> = queue.pending_sequenced().collect();
    entries.sort_by_key(|&(seq, _)| seq);
    entries
        .into_iter()
        .map(|(_, c)| {
            (
                c.completion_time().raw(),
                c.request().length(),
                c.request().holds_slot(),
            )
        })
        .collect()
}

#[test]
fn a_16_kb_put_reads_its_lines_across_its_time() {
    let mut queue = DmaQueue::new();
    let latency = FixedLatency::new(1280);
    queue_transfer(
        &mut queue,
        &latency,
        GuestTicks::new(100),
        &put(0x4000),
        None,
    );
    let parts = queued(&queue);
    assert_eq!(parts.len(), 128);
    for (k, &(at, len, slot)) in parts.iter().enumerate() {
        // Line k ends 128 * (k + 1) bytes in: 1280 * (k + 1) / 128 ticks.
        assert_eq!((at, len, slot), (100 + 10 * (k as u64 + 1), 128, k == 127));
    }
}

#[test]
fn a_put_of_one_line_or_less_stays_whole() {
    let mut queue = DmaQueue::new();
    let latency = FixedLatency::new(1280);
    // 0x40 bytes past a line: as bus requests it would be two parts.
    let range = |at| ByteRange::new(GuestAddr::new(at), 0x80).expect("range fits");
    let off_line = DmaRequest::new(
        DmaDirection::Put,
        range(0x1000),
        range(0x10_0040),
        UnitId::new(1),
    )
    .expect("equal lengths")
    .with_local_store_source();
    queue_transfer(&mut queue, &latency, GuestTicks::new(0), &off_line, None);
    assert_eq!(queued(&queue), [(1280, 0x80, true)]);
}

/// A transfer takes one tick per byte.
struct ByLength;

impl DmaLatencyModel for ByLength {
    fn completion_time(&self, req: &DmaRequest, now: GuestTicks, _queued: &DmaQueue) -> GuestTicks {
        GuestTicks::new(now.raw() + req.length())
    }
}

#[test]
fn a_split_tag_barrier_holds_later_commands_behind_the_commands_before_it() {
    let mut queue = DmaQueue::new();
    let tag = MfcTagId::new(1).expect("tag 1");
    let tagged = |len| put(len).with_tag_id(tag);
    let now = GuestTicks::new(0);
    queue_transfer(&mut queue, &ByLength, now, &tagged(0x10), None);
    let barrier = tagged(0x4000).with_ordering(MfcOrdering::TagBarrier);
    queue_transfer(&mut queue, &ByLength, now, &barrier, None);
    queue_transfer(&mut queue, &ByLength, now, &tagged(0x10), None);
    let times: Vec<u64> = queued(&queue).iter().map(|&(at, _, _)| at).collect();
    assert_eq!(times.len(), 1 + 128 + 1);
    assert_eq!(
        times[1..129].last(),
        Some(&0x4000),
        "the putb ends on its own time"
    );
    assert_eq!(
        times.last(),
        Some(&0x10),
        "the later put waits only for the first"
    );
}

#[test]
fn a_put_with_its_bytes_inline_stays_whole() {
    let mut queue = DmaQueue::new();
    let latency = FixedLatency::new(1280);
    queue_transfer(
        &mut queue,
        &latency,
        GuestTicks::new(0),
        &put(0x200),
        Some(vec![0; 0x200]),
    );
    assert_eq!(queued(&queue), [(1280, 0x200, true)]);
}
