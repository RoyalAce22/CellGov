//! A list's elements hold one command-queue slot between them, and the
//! queue reports the tag groups with a queued stall-and-notify element.

use super::*;
use crate::completion::DmaCompletion;
use crate::request::{DmaDirection, DmaRequest};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::hw::spu::MfcTagId;

const A: UnitId = UnitId::new(1);

fn element(tag: u8) -> DmaRequest {
    let range = ByteRange::new(GuestAddr::new(0x1000), 0x10).unwrap();
    DmaRequest::new(DmaDirection::Get, range, range, A)
        .unwrap()
        .with_tag_id(MfcTagId::new(tag).unwrap())
}

fn queue(requests: &[DmaRequest]) -> DmaQueue {
    let mut q = DmaQueue::new();
    for &r in requests {
        q.enqueue(DmaCompletion::new(r, GuestTicks::new(100)), None);
    }
    q
}

#[test]
fn a_list_holds_one_slot_and_its_tag() {
    let q = queue(&[
        element(2).without_slot(),
        element(2).without_slot(),
        element(2),
        element(4),
    ]);
    assert_eq!(q.issuer_view(A), (2, (1 << 2) | (1 << 4)));
}

#[test]
fn the_stall_view_names_each_group_with_a_flagged_element() {
    let q = queue(&[
        element(2).without_slot(),
        element(2).with_stall_notify().without_slot(),
        element(4),
    ]);
    assert_eq!(q.stall_notify_tags(A), 1 << 2);
    assert_eq!(q.stall_notify_tags(UnitId::new(9)), 0);
}

#[test]
fn each_list_mark_is_a_lane_only_when_set() {
    let plain = queue(&[element(2)]).sync_partial();
    assert_ne!(plain, queue(&[element(2).without_slot()]).sync_partial());
    assert_ne!(
        plain,
        queue(&[element(2).with_stall_notify()]).sync_partial()
    );
    let q = queue(&[element(2).with_stall_notify().without_slot()]);
    assert_eq!(q.sync_partial(), q.sync_partial_from_scratch());
}

#[test]
fn a_refused_list_element_keeps_its_slot_and_stall_marks() {
    let mut q = queue(&[element(2).with_stall_notify().without_slot()]);
    let due = q.process_due_translating(GuestTicks::new(100), |_, _| {
        Some(crate::command::MfcCommandError::DataStorage { ea: 0x1000 })
    });
    assert_eq!(due.raised.len(), 1);
    assert_eq!(q.issuer_view(A), (0, 1 << 2));
    assert_eq!(q.stall_notify_tags(A), 1 << 2);
    assert_eq!(q.sync_partial(), q.sync_partial_from_scratch());
}
