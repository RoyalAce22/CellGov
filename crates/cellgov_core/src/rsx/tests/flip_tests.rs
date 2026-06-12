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
fn state_hash_is_deterministic() {
    let a = RsxFlipState::new();
    let b = RsxFlipState::new();
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_distinguishes_status() {
    let mut a = RsxFlipState::new();
    let mut b = RsxFlipState::new();
    b.request_flip(0);
    assert_ne!(a.state_hash(), b.state_hash());
    a.restore(CELL_GCM_DISPLAY_FLIP_STATUS_WAITING, 0, true, 0);
    assert_eq!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_distinguishes_each_field() {
    fn hash_with(status: u8, handler: u32, pending: bool, buffer_index: u8) -> u64 {
        let mut s = RsxFlipState::new();
        s.restore(status, handler, pending, buffer_index);
        s.state_hash()
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

#[test]
fn empty_flip_state_hash_golden() {
    let s = RsxFlipState::new();
    let got = s.state_hash();
    let mut h = cellgov_mem::Fnv1aHasher::new();
    h.write(&[STATE_HASH_FORMAT_VERSION]);
    h.write(&[CELL_GCM_DISPLAY_FLIP_STATUS_DONE]);
    h.write(&0u32.to_le_bytes());
    h.write(&[0u8]);
    h.write(&[0u8]);
    assert_eq!(got, h.finish());
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
