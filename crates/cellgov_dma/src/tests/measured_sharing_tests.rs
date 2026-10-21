//! The measured model's queue and bus terms: which queued transfers
//! share the bus with a new one, and where a transfer behind its own
//! MFC's tail completes.

use super::*;
use crate::completion::DmaCompletion;
use cellgov_mem::{ByteRange, GuestAddr};

const NOW: GuestTicks = GuestTicks::new(1_000);
const FAR: GuestTicks = GuestTicks::new(1_000_000);

fn request(issuer: u64, size: u64, command: bool) -> DmaRequest {
    let main = ByteRange::new(GuestAddr::new(0x10_0000 + issuer * 0x1_0000), size).expect("main");
    let ls = ByteRange::new(GuestAddr::new(0x1000), size).expect("ls");
    let req = DmaRequest::new(DmaDirection::Get, main, ls, UnitId::new(issuer)).expect("a request");
    if command {
        req.with_command_word(0x40)
    } else {
        req
    }
}

/// Queue `issuer`'s transfer of `size` bytes, completing at `done`.
fn in_flight(queue: &mut DmaQueue, issuer: u64, size: u64, command: bool, done: GuestTicks) {
    queue.enqueue(
        DmaCompletion::new(request(issuer, size, command), done),
        None,
    );
}

/// Issuer 1's 16 KB get beside `queue`, in ticks from `NOW`.
fn get_16k(queue: &DmaQueue) -> u64 {
    MeasuredMfcLatency::new(10)
        .completion_time(&request(1, 0x4000, true), NOW, queue)
        .raw()
        - NOW.raw()
}

#[test]
fn another_spu_s_transfer_in_flight_slows_a_large_get() {
    let alone = get_16k(&DmaQueue::new());
    let mut queue = DmaQueue::new();
    in_flight(&mut queue, 2, 0x4000, true, FAR);
    assert!(
        get_16k(&queue) > alone,
        "{} against {alone}",
        get_16k(&queue)
    );
}

#[test]
fn an_spu_with_two_transfers_in_flight_is_one_sharer() {
    let mut one = DmaQueue::new();
    in_flight(&mut one, 2, 0x4000, true, FAR);
    let mut two = one.clone();
    in_flight(&mut two, 2, 0x4000, true, FAR);
    assert_eq!(get_16k(&two), get_16k(&one));
}

#[test]
fn a_transfer_done_by_now_or_moving_no_mfc_bytes_shares_nothing() {
    let alone = get_16k(&DmaQueue::new());
    for (size, command, done) in [(0x4000, true, NOW), (0x4000, false, FAR), (0, true, FAR)] {
        let mut queue = DmaQueue::new();
        in_flight(&mut queue, 2, size, command, done);
        assert_eq!(
            get_16k(&queue),
            alone,
            "{size} bytes, command {command}, done at {}",
            done.raw()
        );
    }
}

#[test]
fn the_issuer_s_own_transfers_share_nothing() {
    let alone = get_16k(&DmaQueue::new());
    let mut queue = DmaQueue::new();
    in_flight(&mut queue, 1, 0x4000, true, GuestTicks::new(NOW.raw() + 1));
    assert_eq!(get_16k(&queue), alone);
}

#[test]
fn a_small_transfer_behind_its_mfc_s_tail_holds_the_queue_its_least_time() {
    let tail = GuestTicks::new(NOW.raw() + 10_000);
    let mut queue = DmaQueue::new();
    in_flight(&mut queue, 1, 0x4000, true, tail);
    let done = MeasuredMfcLatency::new(10).completion_time(&request(1, 128, true), NOW, &queue);
    let least = tb_to_ticks(MFC_TRANSFER_MIN_TB_SUM, u64::from(MFC_TIMING_RUNS));
    assert_eq!(done.raw(), tail.raw() + least);
}

#[test]
fn a_command_that_moves_no_bytes_waits_for_no_tail() {
    let tail = GuestTicks::new(NOW.raw() + 10_000);
    let mut queue = DmaQueue::new();
    in_flight(&mut queue, 1, 0x4000, true, tail);
    let done = MeasuredMfcLatency::new(10).completion_time(&request(1, 0, true), NOW, &queue);
    assert!(done < tail, "{} against {}", done.raw(), tail.raw());
}
