//! MailboxRegistry access paths and state-hash sensitivity to message contents and order.

use super::*;

#[test]
fn registered_mailboxes_start_empty() {
    let mut r = MailboxRegistry::new();
    let id = r.register(4);
    let m = r.get(id).expect("present");
    assert!(m.is_empty());
    assert_eq!(m.capacity(), 4);
}

#[test]
fn get_mut_lets_caller_send_into_a_mailbox() {
    let mut r = MailboxRegistry::new();
    let id = r.register(4);
    r.get_mut(id).unwrap().force_send(42);
    assert_eq!(r.get(id).unwrap().len(), 1);
    assert_eq!(r.get(id).unwrap().peek(), Some(42));
}

#[test]
fn register_then_try_send_until_full_returns_false() {
    let mut r = MailboxRegistry::new();
    let id = r.register(2);
    let m = r.get_mut(id).unwrap();
    assert!(m.try_send(1));
    assert!(m.try_send(2));
    assert!(!m.try_send(3));
    assert_eq!(m.len(), 2);
}

#[test]
fn state_hash_changes_when_a_mailbox_receives_a_send() {
    let mut r = MailboxRegistry::new();
    let id = r.register(4);
    let h0 = r.state_hash();
    r.get_mut(id).unwrap().force_send(7);
    let h1 = r.state_hash();
    assert_ne!(h0, h1);
}

#[test]
fn state_hash_distinguishes_message_contents() {
    let mut a = MailboxRegistry::new();
    let id_a = a.register(4);
    a.get_mut(id_a).unwrap().force_send(1);

    let mut b = MailboxRegistry::new();
    let id_b = b.register(4);
    b.get_mut(id_b).unwrap().force_send(2);

    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_distinguishes_message_order() {
    let mut a = MailboxRegistry::new();
    let id_a = a.register(4);
    a.get_mut(id_a).unwrap().force_send(1);
    a.get_mut(id_a).unwrap().force_send(2);

    let mut b = MailboxRegistry::new();
    let id_b = b.register(4);
    b.get_mut(id_b).unwrap().force_send(2);
    b.get_mut(id_b).unwrap().force_send(1);

    assert_ne!(a.state_hash(), b.state_hash());
}

#[test]
fn state_hash_round_trips_after_drain() {
    let mut r = MailboxRegistry::new();
    let id = r.register(4);
    let h0 = r.state_hash();
    r.get_mut(id).unwrap().force_send(1);
    r.get_mut(id).unwrap().force_send(2);
    assert_ne!(r.state_hash(), h0);
    assert_eq!(r.get_mut(id).unwrap().try_receive(), Some(1));
    assert_eq!(r.get_mut(id).unwrap().try_receive(), Some(2));
    assert_eq!(r.state_hash(), h0);
}
