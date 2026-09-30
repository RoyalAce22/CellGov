//! Where a transfer into the SPU thread window lands.

use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest, MfcCommandError};
use cellgov_event::UnitId;
use cellgov_exec::SignalNotifier;
use cellgov_lv2::{GroupState, ThreadGroupTable};
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_time::GuestTicks;

use super::{window_target, WindowTarget};

const ISSUER: UnitId = UnitId::new(1);
const PEER: UnitId = UnitId::new(2);
const LONER: UnitId = UnitId::new(3);

/// Slot 1's area: the issuer sits in slot 0 and its peer in slot 1.
const SLOT_1: u64 = 0xF010_0000;

fn groups() -> ThreadGroupTable {
    let mut groups = ThreadGroupTable::new();
    let group = groups.create(2).expect("a group id");
    groups.get_mut(group).expect("the group").state = GroupState::Running;
    groups.record_spu(ISSUER, group, 0).expect("slot 0");
    groups.record_spu(PEER, group, 1).expect("slot 1");
    groups
}

fn transfer(
    issuer: UnitId,
    direction: DmaDirection,
    ea: u64,
    len: u64,
) -> Option<Result<WindowTarget, MfcCommandError>> {
    let main = ByteRange::new(GuestAddr::new(ea), len).expect("range");
    let local = ByteRange::new(GuestAddr::new(0x100), len).expect("range");
    let (src, dst) = match direction {
        DmaDirection::Put => (local, main),
        DmaDirection::Get => (main, local),
    };
    let request = DmaRequest::new(direction, src, dst, issuer).expect("matching sizes");
    window_target(&groups(), &DmaCompletion::new(request, GuestTicks::ZERO))
}

#[test]
fn a_transfer_outside_the_window_or_from_no_group_resolves_in_main_storage() {
    assert_eq!(transfer(ISSUER, DmaDirection::Put, 0xEFFF_FFF0, 16), None);
    assert_eq!(transfer(ISSUER, DmaDirection::Put, 0x1_0000_0000, 16), None);
    assert_eq!(transfer(LONER, DmaDirection::Put, SLOT_1, 16), None);
}

#[test]
fn a_transfer_into_a_slot_local_store_reaches_that_thread() {
    for direction in [DmaDirection::Put, DmaDirection::Get] {
        assert_eq!(
            transfer(ISSUER, direction, SLOT_1 + 0x800, 16),
            Some(Ok(WindowTarget::LocalStore {
                unit: PEER,
                lsa: 0x800
            }))
        );
    }
    assert_eq!(
        transfer(PEER, DmaDirection::Get, 0xF000_0000 + 0x3_FFF0, 16),
        Some(Ok(WindowTarget::LocalStore {
            unit: ISSUER,
            lsa: 0x3_FFF0
        })),
        "the last quadword of slot 0's local store"
    );
    assert_eq!(
        transfer(ISSUER, DmaDirection::Put, 0xF000_0800, 16),
        Some(Ok(WindowTarget::LocalStore {
            unit: ISSUER,
            lsa: 0x800
        })),
        "a thread's own slot is its own local store"
    );
}

/// [CBEA p:102 s:8.7.1], [CBEA p:103 s:8.7.2], [CBEA p:99 s:8.6.2] the problem-state offsets of the two signal registers and the inbound mailbox.
#[test]
fn a_four_byte_put_reaches_the_signal_registers_and_the_inbound_mailbox() {
    for (offset, target) in [
        (
            0x5_400C,
            WindowTarget::Signal {
                unit: PEER,
                register: SignalNotifier::One,
            },
        ),
        (
            0x5_C00C,
            WindowTarget::Signal {
                unit: PEER,
                register: SignalNotifier::Two,
            },
        ),
        (0x4_400C, WindowTarget::InboundMailbox { unit: PEER }),
    ] {
        assert_eq!(
            transfer(ISSUER, DmaDirection::Put, SLOT_1 + offset, 4),
            Some(Ok(target)),
            "offset 0x{offset:x}"
        );
    }
}

#[test]
fn any_other_access_into_a_slot_is_a_data_storage_fault() {
    let refused = |ea| Some(Err(MfcCommandError::DataStorage { ea }));
    for (issuer, direction, ea, len) in [
        // Across the end of the local store.
        (ISSUER, DmaDirection::Put, SLOT_1 + 0x3_FFF0, 32),
        // A get from a register, and a put of another size into one.
        (ISSUER, DmaDirection::Get, SLOT_1 + 0x5_400C, 4),
        (ISSUER, DmaDirection::Put, SLOT_1 + 0x5_4008, 8),
        // A problem-state offset that names no register the model has.
        (ISSUER, DmaDirection::Put, SLOT_1 + 0x4_0000, 4),
        // A slot with no thread.
        (ISSUER, DmaDirection::Put, 0xF020_0000, 16),
    ] {
        assert_eq!(
            transfer(issuer, direction, ea, len),
            refused(ea),
            "0x{ea:x} x {len}"
        );
    }
}

#[test]
fn a_transfer_that_completes_after_its_issuer_stops_still_reaches_the_window() {
    let mut groups = groups();
    groups.notify_spu_finished(ISSUER).expect("a live unit");
    let main = ByteRange::new(GuestAddr::new(SLOT_1), 16).expect("range");
    let local = ByteRange::new(GuestAddr::new(0x100), 16).expect("range");
    let request = DmaRequest::new(DmaDirection::Put, local, main, ISSUER).expect("matching sizes");
    assert_eq!(
        window_target(&groups, &DmaCompletion::new(request, GuestTicks::ZERO)),
        Some(Ok(WindowTarget::LocalStore { unit: PEER, lsa: 0 }))
    );
}
