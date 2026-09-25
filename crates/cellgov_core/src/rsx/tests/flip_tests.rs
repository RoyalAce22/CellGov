//! Display-flip status transitions between WAITING and DONE, plus per-field hash folding.

use super::*;

#[test]
fn new_starts_done_with_nothing_pending() {
    let s = RsxFlipState::new();
    assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_DONE);
    assert_eq!(s.handler(), 0);
    assert!(!s.pending());
    assert_eq!(s.buffer_index(), 0);
}

#[test]
fn request_flip_sets_waiting_and_records_buffer_index() {
    let mut s = RsxFlipState::new();
    s.request_flip(3);
    assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_WAITING);
    assert!(s.pending());
    assert_eq!(s.buffer_index(), 3);
}

#[test]
fn second_request_overwrites_buffer_index_keeps_waiting() {
    let mut s = RsxFlipState::new();
    s.request_flip(1);
    s.request_flip(2);
    assert!(s.pending());
    assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_WAITING);
    assert_eq!(s.buffer_index(), 2);
}

#[test]
fn complete_pending_transitions_to_done_exactly_once() {
    let mut s = RsxFlipState::new();
    s.request_flip(1);
    assert!(s.complete_pending_flip(), "first complete fires");
    assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_DONE);
    assert!(!s.pending());
    assert!(
        !s.complete_pending_flip(),
        "second complete is a no-op (nothing pending)"
    );
}

#[test]
fn complete_pending_on_fresh_state_is_noop() {
    let mut s = RsxFlipState::new();
    assert!(!s.complete_pending_flip());
    assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_DONE);
}

#[test]
fn set_handler_records_address_without_touching_status() {
    let mut s = RsxFlipState::new();
    s.set_handler(0xDEAD_BEEF);
    assert_eq!(s.handler(), 0xDEAD_BEEF);
    assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_DONE);
    assert!(!s.pending());
}

#[test]
fn restore_overwrites_all_fields() {
    let mut s = RsxFlipState::new();
    s.restore(CELL_GCM_DISPLAY_FLIP_STATUS_WAITING, 0x1234_5678, true, 7);
    assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_WAITING);
    assert_eq!(s.handler(), 0x1234_5678);
    assert!(s.pending());
    assert_eq!(s.buffer_index(), 7);
}

#[test]
fn sync_term_is_deterministic() {
    let a = RsxFlipState::new();
    let b = RsxFlipState::new();
    assert_eq!(a.sync_term(), b.sync_term());
}

#[test]
fn sync_term_distinguishes_status() {
    let mut a = RsxFlipState::new();
    let mut b = RsxFlipState::new();
    b.request_flip(0);
    assert_ne!(a.sync_term(), b.sync_term());
    a.restore(CELL_GCM_DISPLAY_FLIP_STATUS_WAITING, 0, true, 0);
    assert_eq!(a.sync_term(), b.sync_term());
}

#[test]
fn sync_term_distinguishes_each_field() {
    fn hash_with(status: u8, handler: u32, pending: bool, buffer_index: u8) -> u128 {
        let mut s = RsxFlipState::new();
        s.restore(status, handler, pending, buffer_index);
        s.sync_term()
    }
    let base = hash_with(0, 0, false, 0);
    assert_ne!(base, hash_with(1, 0, false, 0), "status field folds in");
    assert_ne!(base, hash_with(0, 1, false, 0), "handler field folds in");
    assert_ne!(base, hash_with(0, 0, true, 0), "pending field folds in");
    assert_ne!(
        base,
        hash_with(0, 0, false, 1),
        "buffer_index field folds in"
    );
}

/// Computed outside the crate from the SplitMix64 key stream of the
/// sync-state lanes.
#[test]
fn flip_term_wire_format_golden() {
    assert_eq!(
        RsxFlipState::new().sync_term(),
        0x4e7e_4ff1_bb1f_4aad_66be_3e92_de27_9cc5
    );
    let mut s = RsxFlipState::new();
    s.restore(CELL_GCM_DISPLAY_FLIP_STATUS_WAITING, 0x1234_5678, true, 7);
    assert_eq!(s.sync_term(), 0xa6c4_e9ae_bc7f_1bf6_1fc0_0b8f_e0c3_0f81);
}

#[test]
fn complete_pending_returns_false_after_self_sequence() {
    let mut s = RsxFlipState::new();
    for i in 0..5u8 {
        s.request_flip(i);
        assert!(s.pending());
        assert!(s.complete_pending_flip());
        assert_eq!(s.status(), CELL_GCM_DISPLAY_FLIP_STATUS_DONE);
        assert!(!s.pending());
        assert_eq!(
            s.buffer_index(),
            i,
            "buffer_index sticks across DONE; guest must not read it stale"
        );
    }
}
