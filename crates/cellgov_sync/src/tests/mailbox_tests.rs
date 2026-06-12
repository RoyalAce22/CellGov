//! Mailbox FIFO semantics -- capacity limits, force-send overrun, peek, and iteration.

use super::*;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn hash<T: Hash>(t: &T) -> u64 {
    let mut h = DefaultHasher::new();
    t.hash(&mut h);
    h.finish()
}

#[test]
fn id_roundtrip() {
    assert_eq!(MailboxId::new(42).raw(), 42);
}

#[test]
fn id_hash_matches_eq() {
    assert_eq!(hash(&MailboxId::new(7)), hash(&MailboxId::new(7)));
    assert_ne!(hash(&MailboxId::new(7)), hash(&MailboxId::new(8)));
}

#[test]
fn id_copy_preserves_value() {
    let a = MailboxId::new(5);
    let b = a;
    assert_eq!(a, b);
    assert_eq!(a.raw(), 5);
}

#[test]
fn id_display_emits_raw_integer() {
    assert_eq!(format!("{}", MailboxId::new(42)), "42");
}

#[test]
fn id_max_roundtrips() {
    assert_eq!(MailboxId::new(u64::MAX).raw(), u64::MAX);
}

#[test]
fn new_mailbox_is_empty_and_has_full_capacity() {
    let m = Mailbox::with_capacity(4);
    assert!(m.is_empty());
    assert_eq!(m.len(), 0);
    assert_eq!(m.peek(), None);
    assert_eq!(m.capacity(), 4);
    assert_eq!(m.remaining_capacity(), 4);
    assert!(!m.is_full());
}

#[test]
fn try_send_then_try_receive_returns_in_fifo_order() {
    let mut m = Mailbox::with_capacity(4);
    assert!(m.try_send(1));
    assert!(m.try_send(2));
    assert!(m.try_send(3));
    assert_eq!(m.len(), 3);
    assert_eq!(m.try_receive(), Some(1));
    assert_eq!(m.try_receive(), Some(2));
    assert_eq!(m.try_receive(), Some(3));
    assert_eq!(m.try_receive(), None);
    assert!(m.is_empty());
}

#[test]
fn try_send_returns_false_when_full() {
    let mut m = Mailbox::with_capacity(2);
    assert!(m.try_send(10));
    assert!(m.try_send(20));
    assert!(m.is_full());
    assert!(!m.try_send(30));
    // Queue unchanged on full.
    assert_eq!(m.len(), 2);
    assert_eq!(m.try_receive(), Some(10));
    assert_eq!(m.try_receive(), Some(20));
}

#[test]
fn force_send_overruns_oldest_on_full() {
    let mut m = Mailbox::with_capacity(2);
    m.force_send(1);
    m.force_send(2);
    m.force_send(3); // overrun: drops 1
    assert_eq!(m.len(), 2);
    assert_eq!(m.try_receive(), Some(2));
    assert_eq!(m.try_receive(), Some(3));
}

#[test]
fn force_send_no_overrun_when_room() {
    let mut m = Mailbox::with_capacity(4);
    m.force_send(1);
    m.force_send(2);
    assert_eq!(m.len(), 2);
    assert_eq!(m.try_receive(), Some(1));
}

#[test]
fn try_receive_on_empty_returns_none() {
    let mut m = Mailbox::with_capacity(4);
    assert_eq!(m.try_receive(), None);
    assert_eq!(m.try_receive(), None);
    assert!(m.is_empty());
}

#[test]
fn peek_does_not_consume() {
    let mut m = Mailbox::with_capacity(4);
    assert!(m.try_send(0xdead_beef));
    assert_eq!(m.peek(), Some(0xdead_beef));
    assert_eq!(m.peek(), Some(0xdead_beef));
    assert_eq!(m.len(), 1);
    assert_eq!(m.try_receive(), Some(0xdead_beef));
    assert_eq!(m.peek(), None);
}

#[test]
fn remaining_capacity_tracks_occupancy() {
    let mut m = Mailbox::with_capacity(4);
    assert_eq!(m.remaining_capacity(), 4);
    assert!(m.try_send(1));
    assert_eq!(m.remaining_capacity(), 3);
    m.try_receive();
    assert_eq!(m.remaining_capacity(), 4);
}

#[test]
fn interleaved_send_and_receive_preserves_order() {
    let mut m = Mailbox::with_capacity(4);
    assert!(m.try_send(10));
    assert_eq!(m.try_receive(), Some(10));
    assert!(m.try_send(20));
    assert!(m.try_send(30));
    assert_eq!(m.try_receive(), Some(20));
    assert!(m.try_send(40));
    assert_eq!(m.try_receive(), Some(30));
    assert_eq!(m.try_receive(), Some(40));
    assert_eq!(m.try_receive(), None);
}

#[test]
fn clone_is_independent() {
    let mut a = Mailbox::with_capacity(4);
    assert!(a.try_send(1));
    assert!(a.try_send(2));
    let mut b = a.clone();
    a.try_receive();
    assert_eq!(a.len(), 1);
    assert_eq!(b.len(), 2);
    assert_eq!(b.try_receive(), Some(1));
    assert_eq!(b.try_receive(), Some(2));
}

#[test]
fn iter_walks_front_to_back_without_consuming() {
    let mut m = Mailbox::with_capacity(4);
    assert!(m.try_send(10));
    assert!(m.try_send(20));
    assert!(m.try_send(30));
    let collected: Vec<u32> = m.iter().copied().collect();
    assert_eq!(collected, vec![10, 20, 30]);
    assert_eq!(m.len(), 3);
}

#[test]
fn iter_on_empty_yields_nothing() {
    let m = Mailbox::with_capacity(4);
    assert_eq!(m.iter().count(), 0);
}
