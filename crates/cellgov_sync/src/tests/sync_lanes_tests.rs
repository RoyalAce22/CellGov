//! The mailbox, signal and reservation partials of the sync-state sum.

use crate::{MailboxRegistry, ReservationTable, ReservedLine, SignalRegistry};
use cellgov_event::UnitId;

fn unit(raw: u64) -> UnitId {
    UnitId::new(raw)
}

/// Computed outside the crate: presence, length 1 and message 7 of
/// mailbox 0, under the sync-state keys.
#[test]
fn mailbox_partial_wire_format_golden() {
    let mut r = MailboxRegistry::new();
    let id = r.register(4);
    assert!(r.get_mut(id).unwrap().try_send(7));
    assert_eq!(r.sync_partial(), 0x3da6_3fa3_2115_c648_47c3_6d98_4268_2429);
}

/// Computed outside the crate: presence and line 0x1000 of unit 2 in
/// address space 5.
#[test]
fn reservation_partial_wire_format_golden() {
    let mut t = ReservationTable::in_space(5);
    t.insert_or_replace(unit(2), ReservedLine::containing(0x1000));
    assert_eq!(t.sync_partial(), 0x7c21_0d35_d4dc_ce78_afa7_217f_3adf_8ece);
}

#[test]
fn every_mailbox_change_moves_the_partial() {
    let mut r = MailboxRegistry::new();
    let empty = r.sync_partial();
    let id = r.register(2);
    let registered = r.sync_partial();
    assert_ne!(
        registered, empty,
        "a registered empty mailbox differs from none"
    );
    assert!(r.get_mut(id).unwrap().try_send(0));
    let one_zero = r.sync_partial();
    assert_ne!(
        one_zero, registered,
        "a queued zero message moves the length lane"
    );
    assert!(r.get_mut(id).unwrap().try_send(5));
    let two = r.sync_partial();
    assert_ne!(two, one_zero);
    r.get_mut(id).unwrap().force_send(6);
    assert_ne!(r.sync_partial(), two, "an overrun shifts the queue");
    r.get_mut(id).unwrap().try_receive();
    r.get_mut(id).unwrap().try_receive();
    assert_eq!(
        r.sync_partial(),
        registered,
        "a drained mailbox returns its partial"
    );
}

#[test]
fn every_signal_change_moves_the_partial() {
    let mut r = SignalRegistry::new();
    let id = r.register();
    let zero = r.sync_partial();
    r.get_mut(id).unwrap().or_in(0b10);
    let set = r.sync_partial();
    assert_ne!(set, zero);
    r.get_mut(id).unwrap().clear();
    assert_eq!(r.sync_partial(), zero);
}

#[test]
fn every_reservation_change_moves_the_partial() {
    let mut t = ReservationTable::new();
    let empty = t.sync_partial();
    t.insert_or_replace(unit(1), ReservedLine::containing(0x100));
    let held = t.sync_partial();
    assert_ne!(held, empty);
    t.insert_or_replace(unit(1), ReservedLine::containing(0x200));
    assert_ne!(t.sync_partial(), held);
    t.insert_or_replace(unit(1), ReservedLine::containing(0x100));
    assert_eq!(t.sync_partial(), held);
    assert_eq!(t.clear_covering(0x100, 4, None), 1);
    assert_eq!(
        t.sync_partial(),
        empty,
        "insert then clear returns the partial"
    );
    t.insert_or_replace(unit(3), ReservedLine::containing(0));
    assert_ne!(
        t.sync_partial(),
        empty,
        "a reservation of line 0 differs from none"
    );
    t.remove_if_present(unit(3));
    assert_eq!(t.sync_partial(), empty);
}

#[test]
fn two_entries_that_exchange_contents_hash_differently() {
    let pair = |a: u32, b: u32| {
        let mut r = MailboxRegistry::new();
        for message in [a, b] {
            let id = r.register(4);
            assert!(r.get_mut(id).unwrap().try_send(message));
        }
        r.sync_partial()
    };
    assert_ne!(pair(1, 2), pair(2, 1));
    let lines = |a: u64, b: u64| {
        let mut t = ReservationTable::new();
        t.insert_or_replace(unit(0), ReservedLine::containing(a));
        t.insert_or_replace(unit(1), ReservedLine::containing(b));
        t.sync_partial()
    };
    assert_ne!(lines(0x80, 0x100), lines(0x100, 0x80));
}

#[test]
fn one_table_in_two_spaces_hashes_differently() {
    let held = |space| {
        let mut t = ReservationTable::in_space(space);
        t.insert_or_replace(unit(1), ReservedLine::containing(0x80));
        t.sync_partial()
    };
    assert_ne!(held(0), held(1));
    assert_eq!(held(0), {
        let mut t = ReservationTable::new();
        t.insert_or_replace(unit(1), ReservedLine::containing(0x80));
        t.sync_partial()
    });
}

/// Each kept partial equals a rebuild from every entry after every
/// mutating path, in both build profiles.
#[test]
fn the_kept_partials_track_every_mutating_path() {
    let mut mailboxes = MailboxRegistry::new();
    let a = mailboxes.register(4);
    let check = |m: &MailboxRegistry| assert_eq!(m.sync_partial(), m.sync_partial_from_scratch());
    check(&mailboxes);
    assert!(mailboxes.register_at(crate::MailboxId::new(3), 1));
    check(&mailboxes);
    assert!(mailboxes.get_mut(a).unwrap().try_send(9));
    check(&mailboxes);
    let _ = mailboxes.get_mut(a).unwrap().try_receive();
    check(&mailboxes);
    mailboxes.get_mut(a).unwrap().force_send(1);
    check(&mailboxes);
    let copy = mailboxes.clone();
    assert_eq!(copy.sync_partial(), mailboxes.sync_partial());

    let mut signals = SignalRegistry::new();
    let s = signals.register();
    assert_eq!(signals.sync_partial(), signals.sync_partial_from_scratch());
    signals.get_mut(s).unwrap().or_in(4);
    assert_eq!(signals.sync_partial(), signals.sync_partial_from_scratch());

    let mut t = ReservationTable::in_space(2);
    for u in 0..6 {
        t.insert_or_replace(unit(u), ReservedLine::containing(u * 0x80));
    }
    t.insert_or_replace(unit(2), ReservedLine::containing(0x4000));
    t.remove_if_present(unit(4));
    t.clear_covering(0, 0x100, Some(unit(1)));
    assert_eq!(t.sync_partial(), t.sync_partial_from_scratch());
}
