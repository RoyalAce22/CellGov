use super::*;
use crate::{answered_numbers, AddressSpace, AddressSpaceChange, Hypervisor};
use cellgov_event::UnitId;
use cellgov_mem::lanes::{contribution, LaneIndex};
use cellgov_ps3_abi::lv1::call::{
    CONFIGURE_IRQ_STATE_BITMAP, CONNECT_IRQ_PLUG_EXT, CONSTRUCT_EVENT_RECEIVE_PORT, PAUSE,
};
use cellgov_time::GuestTicks;

use crate::group::interrupt::pause::PausedThread;
use crate::group::spe::LogicalSpe;
use crate::group::state::StateChange;

/// The `sc` of the kernel's port call in the setup of a thread's record
/// [OFW-4.93 lv2_kernel a:0x8000000000007d8c]. The console gave thread 1's
/// call the port id 4 (`GAME_OS_PORT_CALLS`).
const RECORD_SC: u64 = 0x8000_0000_0000_7d8c;

/// The port of thread 1's record setup.
const PORT: u64 = 4;

/// The `sc` of the kernel's event send
/// [OFW-4.93 lv2_kernel a:0x8000000000025a34].
const SEND_SC: u64 = 0x8000_0000_0002_5a34;

/// A bitmap address in the real mode region, 8-byte aligned.
const THREAD_1_BITMAP: u64 = 0x0036_9200;

/// The plug that the tests connect the port to.
const PLUG: u64 = 9;

/// Plug 9 is bit 63 - 9 of status doubleword 0 (`plug_status_bit`).
const PLUG_BIT: u64 = 1 << 54;

fn call(number: u64, args: &[u64], thread: u8) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..args.len()].copy_from_slice(args);
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc: SEND_SC,
        lr: 0,
        thread: Some(thread),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

fn send(id: u64, thread: u8) -> HypercallRequest {
    call(SEND_EVENT_LOCALLY, &[id], thread)
}

fn storage(mask: u64) -> GuestMemory {
    storage_at(0, mask)
}

/// Storage whose thread 1 bitmap holds `mask` in mask doubleword `index`.
fn storage_at(index: u64, mask: u64) -> GuestMemory {
    let mut storage = GuestMemory::new(0x0037_0000);
    let address = THREAD_1_BITMAP + IRQ_MASK_OFFSET + index * 8;
    let range = ByteRange::new(GuestAddr::new(address), 8).expect("the mask doubleword");
    storage
        .apply_commit(range, &mask.to_be_bytes())
        .expect("the mask write");
    storage
}

fn thread_1() -> IrqStateBitmapKey {
    IrqStateBitmapKey {
        ppe_id: 1,
        thread: 1,
    }
}

fn answered(disposition: HypercallDisposition) -> HypercallAnswer {
    match disposition {
        HypercallDisposition::Answered(answer) => answer,
        refused => panic!("{refused:?}"),
    }
}

/// A 4.93 hypervisor after the kernel's thread 1 registers its bitmap and
/// constructs [`PORT`]. Thread 0 connects the port to [`PLUG`] of thread
/// 1 when `connect` is set, and thread 1 then pauses in mode 0 when
/// `pause` is set.
fn booted(connect: bool, pause: bool, storage: &GuestMemory) -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let bitmap = call(CONFIGURE_IRQ_STATE_BITMAP, &[1, 1, THREAD_1_BITMAP], 1);
    answered(hv.dispatch_with(&bitmap, storage));
    let port = HypercallRequest {
        pc: RECORD_SC,
        ..call(CONSTRUCT_EVENT_RECEIVE_PORT, &[], 1)
    };
    assert_eq!(
        answered(hv.dispatch_with(&port, storage))
            .outputs()
            .map(|o| o[0]),
        Some(PORT)
    );
    if connect {
        let plug = call(CONNECT_IRQ_PLUG_EXT, &[1, 1, PLUG, PORT, 0], 0);
        assert_eq!(answered(hv.dispatch_with(&plug, storage)).r3, 0);
    }
    if pause {
        answered(hv.dispatch_with(&call(PAUSE, &[0], 1), storage));
    }
    hv
}

fn success() -> HypercallAnswer {
    HypercallAnswer::lv1_status(0, [0; 7])
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: SEND_EVENT_LOCALLY,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

/// The answer of a raise of [`PLUG`] on paused thread 1: the pending
/// event, the status write and the raised shadow doubleword.
fn raised(woken: bool) -> HypercallDisposition {
    raise_answer(PORT, 0, PLUG_BIT, woken)
}

/// The answer of a raise on paused thread 1 through the port `id`: the
/// pending event, the write of `value` to status doubleword `index`, and
/// that shadow doubleword.
fn raise_answer(id: u64, index: u8, value: u64, woken: bool) -> HypercallDisposition {
    let mut answer = success();
    answer.push_change(InterruptChange::EventPending(id));
    let status = THREAD_1_BITMAP + u64::from(index) * 8;
    answer.effects.push(Effect::SharedWriteIntent {
        range: ByteRange::new(GuestAddr::new(status), 8).expect("the status doubleword"),
        bytes: WritePayload::new(value.to_be_bytes().to_vec()),
        source: UnitId::new(1),
        source_time: GuestTicks::ZERO,
    });
    answer.push_change(InterruptChange::StatusShadow(StatusShadowWrite {
        key: thread_1(),
        index,
        value,
    }));
    if woken {
        answer.push_change(InterruptChange::ThreadWoken(1));
    }
    HypercallDisposition::Answered(answer)
}

#[test]
fn the_hypervisor_answers_24() {
    assert!(answered_numbers().contains(&SEND_EVENT_LOCALLY));
}

#[test]
fn an_unconnected_port_gets_r3_0_and_keeps_the_pending_event() {
    let storage = storage(0);
    let mut hv = booted(false, false, &storage);
    let before = hv.sync_term();
    let mut expected = success();
    expected.push_change(InterruptChange::EventPending(PORT));
    assert_eq!(
        hv.dispatch_with(&send(PORT, 0), &storage),
        HypercallDisposition::Answered(expected)
    );
    assert!(hv.event_receive_ports().is_pending(PORT));
    assert_ne!(hv.sync_term(), before);
}

#[test]
fn a_send_on_a_pending_port_gets_r3_0_and_changes_nothing() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(true, true, &storage);
    hv.dispatch_with(&send(PORT, 0), &storage);
    let once = hv.clone();
    assert_eq!(
        hv.dispatch_with(&send(PORT, 0), &storage),
        HypercallDisposition::Answered(success())
    );
    assert_eq!(hv, once);
}

#[test]
fn an_unmasked_raise_on_a_paused_thread_writes_its_status_and_wakes_it() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(true, true, &storage);
    assert_eq!(hv.dispatch_with(&send(PORT, 0), &storage), raised(true));
    let interrupt = hv.groups().interrupt();
    assert_eq!(interrupt.bitmaps().shadow(thread_1()), [PLUG_BIT, 0, 0, 0]);
    assert!(interrupt.paused().is_woken(1));
    assert_eq!(
        interrupt.paused().thread(1),
        Some(PausedThread {
            thread: 1,
            decrementer_wakes: true,
        })
    );
}

#[test]
fn a_masked_raise_on_a_paused_thread_writes_its_status_and_leaves_it_paused() {
    for mask in [0, !PLUG_BIT] {
        let storage = storage(mask);
        let mut hv = booted(true, true, &storage);
        assert_eq!(
            hv.dispatch_with(&send(PORT, 0), &storage),
            raised(false),
            "{mask:#x}"
        );
        assert!(!hv.groups().interrupt().paused().is_woken(1), "{mask:#x}");
    }
}

#[test]
fn a_raise_on_a_set_shadow_bit_keeps_only_the_pending_event() {
    let storage = storage(PLUG_BIT);
    let hv = booted(true, true, &storage);
    let mut bitmaps = hv.groups().interrupt().bitmaps().clone();
    bitmaps.write_shadow(StatusShadowWrite {
        key: thread_1(),
        index: 0,
        value: PLUG_BIT,
    });
    let interrupt = hv.groups().interrupt();
    let state = SendState {
        spaces: &AddressSpaceState::default(),
        spes: &LogicalSpeState::default(),
        bitmaps: &bitmaps,
        ports: interrupt.ports(),
        paused: interrupt.paused(),
        storage: &storage,
    };
    let mut expected = success();
    expected.push_change(InterruptChange::EventPending(PORT));
    assert_eq!(
        send_event(&send(PORT, 0), Some("4.93"), &state),
        HypercallDisposition::Answered(expected)
    );
}

#[test]
fn a_raise_on_a_running_thread_is_a_refusal_and_changes_nothing() {
    let storage = storage(PLUG_BIT);
    for (caller, what) in [
        (
            1,
            "an event on a plug of the calling thread, whose return lv1 checks for an external interrupt",
        ),
        (
            0,
            "an event on a plug of a running thread, which lv1 signals with an interprocessor interrupt",
        ),
    ] {
        let mut hv = booted(true, false, &storage);
        let before = hv.clone();
        let r = send(PORT, caller);
        assert_eq!(hv.dispatch_with(&r, &storage), refused(&r, what), "{caller}");
        assert_eq!(hv.groups(), before.groups(), "{caller}");
    }
}

#[test]
fn a_raise_with_no_storage_for_the_mask_is_a_refusal() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(true, true, &storage);
    let r = send(PORT, 0);
    assert_eq!(
        hv.dispatch_with(&r, &GuestMemory::new(THREAD_1_BITMAP as usize + 8)),
        refused(
            &r,
            "an event on a plug whose status and mask doublewords CellGov holds no storage for"
        )
    );
}

/// The `sc` of call A of a thread's start
/// [OFW-4.93 lv2_kernel a:0x80000000002656d0]. The console gave thread
/// 1's call the port id 5 (`GAME_OS_PORT_CALLS`).
const SECOND_PORT_SC: u64 = 0x8000_0000_0026_56d0;

/// A 4.93 hypervisor where thread 1 constructs one port for each of
/// `plugs` (at most two), thread 0 connects each port to its plug of
/// thread 1, and thread 1 then pauses in mode 0. Thread 1 first registers
/// its bitmap when `bitmap` is set. The port ids are in plug order.
fn paused_on(plugs: &[u64], bitmap: bool, storage: &GuestMemory) -> (Hypervisor, Vec<u64>) {
    assert!(plugs.len() <= 2, "two port sites only");
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    if bitmap {
        let r = call(CONFIGURE_IRQ_STATE_BITMAP, &[1, 1, THREAD_1_BITMAP], 1);
        answered(hv.dispatch_with(&r, storage));
    }
    let mut ids = Vec::new();
    for (&plug, pc) in plugs.iter().zip([RECORD_SC, SECOND_PORT_SC]) {
        let port = HypercallRequest {
            pc,
            ..call(CONSTRUCT_EVENT_RECEIVE_PORT, &[], 1)
        };
        let id = answered(hv.dispatch_with(&port, storage))
            .outputs()
            .expect("r4")[0];
        let connect = call(CONNECT_IRQ_PLUG_EXT, &[1, 1, plug, id, 0], 0);
        assert_eq!(answered(hv.dispatch_with(&connect, storage)).r3, 0);
        ids.push(id);
    }
    answered(hv.dispatch_with(&call(PAUSE, &[0], 1), storage));
    (hv, ids)
}

/// Plug 200 is bit 63 - 8 of status doubleword 3 (`plug_status_bit`), so
/// lv1 stores at +24 of the bitmap and reads the mask at +56.
#[test]
fn a_raise_on_a_plug_past_doubleword_0_writes_and_masks_its_own_doubleword() {
    const PLUG_200_BIT: u64 = 1 << 55;
    for (mask_index, woken) in [(3, true), (0, false)] {
        let storage = storage_at(mask_index, PLUG_200_BIT);
        let (mut hv, ids) = paused_on(&[200], true, &storage);
        assert_eq!(
            hv.dispatch_with(&send(ids[0], 0), &storage),
            raise_answer(ids[0], 3, PLUG_200_BIT, woken),
            "{mask_index}"
        );
        assert_eq!(
            hv.groups().interrupt().bitmaps().shadow(thread_1()),
            [0, 0, 0, PLUG_200_BIT],
            "{mask_index}"
        );
    }
}

/// lv1 reads the mask only while the pause flag is set, and its first
/// wake clears that flag.
#[test]
fn a_second_unmasked_raise_on_a_woken_thread_carries_no_second_wake() {
    const PLUG_10_BIT: u64 = 1 << 53;
    let storage = storage(PLUG_BIT | PLUG_10_BIT);
    let (mut hv, ids) = paused_on(&[PLUG, 10], true, &storage);
    assert_eq!(
        hv.dispatch_with(&send(ids[0], 0), &storage),
        raise_answer(ids[0], 0, PLUG_BIT, true)
    );
    assert_eq!(
        hv.dispatch_with(&send(ids[1], 0), &storage),
        raise_answer(ids[1], 0, PLUG_BIT | PLUG_10_BIT, false)
    );
    assert!(hv.groups().interrupt().paused().is_woken(1));
}

/// lv1 wakes the thread only for a set mask bit of the raised plug
/// [OFW-4.93 lv1 a:0x2C6694-0x2C66DC]. A bit that an earlier raise left
/// in the shadow gives no wake. The kernel can unmask plug 9 between the
/// two sends.
#[test]
fn a_masked_raise_carries_no_wake_for_an_unmasked_bit_already_in_the_shadow() {
    const PLUG_10_BIT: u64 = 1 << 53;
    let masked = storage(0);
    let (mut hv, ids) = paused_on(&[PLUG, 10], true, &masked);
    assert_eq!(
        hv.dispatch_with(&send(ids[0], 0), &masked),
        raise_answer(ids[0], 0, PLUG_BIT, false)
    );
    let plug_9_unmasked = storage(PLUG_BIT);
    assert_eq!(
        hv.dispatch_with(&send(ids[1], 0), &plug_9_unmasked),
        raise_answer(ids[1], 0, PLUG_BIT | PLUG_10_BIT, false)
    );
    assert!(!hv.groups().interrupt().paused().is_woken(1));
}

#[test]
fn a_raise_on_a_paused_thread_with_no_bitmap_is_a_refusal_and_changes_nothing() {
    let storage = storage(PLUG_BIT);
    let (mut hv, ids) = paused_on(&[PLUG], false, &storage);
    let before = hv.clone();
    let r = send(ids[0], 0);
    assert_eq!(
        hv.dispatch_with(&r, &storage),
        refused(
            &r,
            "an event on a plug of a thread that registered no interrupt state bitmap"
        )
    );
    assert_eq!(hv.groups(), before.groups());
}

#[test]
fn the_logical_ppe_id_gets_no_entry_and_an_unknown_id_is_a_refusal() {
    let storage = storage(0);
    let mut hv = booted(false, false, &storage);
    let ppe_id = game_os_logical_ppe_id("4.93").expect("the 4.93 logical PPE id");
    assert_eq!(
        hv.dispatch_with(&send(ppe_id, 0), &storage),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(SEND_EVENT_NO_ENTRY_R3, [0; 7]))
    );
    let r = send(0x55, 0);
    assert_eq!(
        hv.dispatch_with(&r, &storage),
        refused(
            &r,
            "an event send whose port id names no object CellGov holds"
        )
    );
}

/// lv1 reads the SPE class at +48 of a logical SPE, and refuses it as a
/// port [OFW-4.93 lv1 a:0x2E27CC-0x2E27D8].
#[test]
fn a_logical_spe_id_gets_no_entry_and_no_change() {
    const BOOT_SPE_ID: u64 = 11;
    let storage = storage(0);
    let hv = booted(true, true, &storage);
    let interrupt = hv.groups().interrupt();
    let mut spes = LogicalSpeState::default();
    spes.construct(LogicalSpe {
        id: BOOT_SPE_ID,
        spe_type: 1,
        vas_id: 2,
        area: 0x0000_4c00_0028_0000,
        shadow: 0x0000_3000_0000_6000,
        shadow_real: 0x1100_0000,
    });
    let state = SendState {
        spaces: &AddressSpaceState::default(),
        spes: &spes,
        bitmaps: interrupt.bitmaps(),
        ports: interrupt.ports(),
        paused: interrupt.paused(),
        storage: &storage,
    };
    assert_eq!(
        send_event(&send(BOOT_SPE_ID, 0), Some("4.93"), &state),
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(SEND_EVENT_NO_ENTRY_R3, [0; 7]))
    );
}

/// An irq outlet holds another word at +136, so lv1 finds no port at its
/// id [OFW-4.93 lv1 a:0x2E27DC-0x2E27E8]. An address space is an object
/// of another class.
#[test]
fn an_irq_outlet_id_and_an_address_space_id_get_no_entry() {
    const OUTLET_ID: u64 = 0x30;
    const VAS_ID: u64 = 2;
    let storage = storage(0);
    let hv = booted(false, false, &storage);
    let interrupt = hv.groups().interrupt();
    let mut ports = interrupt.ports().clone();
    ports.construct_outlet(crate::EventReceivePort::new(OUTLET_ID));
    let mut spaces = AddressSpaceState::default();
    spaces.apply(AddressSpaceChange::Constructed(AddressSpace {
        vas_id: VAS_ID,
        htab_size_exp: 18,
        page_size_count: 2,
        page_shifts: 0,
        mapping: None,
    }));
    let state = SendState {
        spaces: &spaces,
        spes: &LogicalSpeState::default(),
        bitmaps: interrupt.bitmaps(),
        ports: &ports,
        paused: interrupt.paused(),
        storage: &storage,
    };
    for id in [OUTLET_ID, VAS_ID] {
        assert_eq!(
            send_event(&send(id, 0), Some("4.93"), &state),
            HypercallDisposition::Answered(HypercallAnswer::lv1_status(
                SEND_EVENT_NO_ENTRY_R3,
                [0; 7]
            )),
            "{id}"
        );
    }
}

#[test]
fn a_send_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let storage = storage(0);
    let hv = booted(false, false, &storage);
    let interrupt = hv.groups().interrupt();
    let state = SendState {
        spaces: &AddressSpaceState::default(),
        spes: &LogicalSpeState::default(),
        bitmaps: interrupt.bitmaps(),
        ports: interrupt.ports(),
        paused: interrupt.paused(),
        storage: &storage,
    };
    let r = send(PORT, 0);
    for firmware in [None, Some("4.92"), Some("3.70")] {
        assert_eq!(
            send_event(&r, firmware, &state),
            refused(
                &r,
                "an event send at a firmware whose lv1 handler is not read"
            ),
            "{firmware:?}"
        );
    }
}

#[test]
fn a_bitmap_registration_writes_the_raised_shadow() {
    let storage = storage(PLUG_BIT);
    let mut hv = booted(true, true, &storage);
    hv.dispatch_with(&send(PORT, 0), &storage);
    let again = call(CONFIGURE_IRQ_STATE_BITMAP, &[1, 1, THREAD_1_BITMAP], 0);
    let answer = answered(hv.dispatch_with(&again, &storage));
    let mut shadow = vec![0u8; 32];
    shadow[..8].copy_from_slice(&PLUG_BIT.to_be_bytes());
    assert!(matches!(
        answer.effects.as_slice(),
        [Effect::SharedWriteIntent { bytes, .. }] if bytes.bytes() == shadow
    ));
}

#[test]
fn a_connect_of_a_pending_port_is_a_refusal() {
    let storage = storage(0);
    let mut hv = booted(false, false, &storage);
    hv.dispatch_with(&send(PORT, 0), &storage);
    let plug = call(CONNECT_IRQ_PLUG_EXT, &[1, 1, PLUG, PORT, 0], 0);
    assert_eq!(
        hv.dispatch_with(&plug, &storage),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: CONNECT_IRQ_PLUG_EXT,
            unit: plug.unit,
            pc: plug.pc,
            what: "an IRQ plug call for a port with a pending event, which lv1 raises on the plug",
        })
    );
}

#[test]
fn a_connect_on_another_plug_keeps_the_raised_bit_and_carries_no_shadow_change() {
    let storage = storage(0);
    let mut hv = booted(true, true, &storage);
    hv.dispatch_with(&send(PORT, 0), &storage);
    // A second port on plug 10 of thread 1: plug 9 still holds its bit.
    let port = HypercallRequest {
        pc: SECOND_PORT_SC,
        ..call(CONSTRUCT_EVENT_RECEIVE_PORT, &[], 1)
    };
    let id = answered(hv.dispatch_with(&port, &storage))
        .outputs()
        .expect("r4")[0];
    let plug = call(CONNECT_IRQ_PLUG_EXT, &[1, 1, 10, id, 0], 0);
    let answer = answered(hv.dispatch_with(&plug, &storage));
    assert!(matches!(
        answer.effects.as_slice(),
        [Effect::SharedWriteIntent { bytes, .. }] if bytes.bytes() == PLUG_BIT.to_be_bytes()
    ));
    assert!(!answer
        .changes
        .iter()
        .any(|c| matches!(c, StateChange::Interrupt(change) if matches!(**change, InterruptChange::StatusShadow(_)))));
}

#[test]
fn each_new_surface_moves_the_term_and_an_empty_one_adds_none() {
    let key = thread_1();
    let mut bitmaps = IrqStateBitmapState::default();
    let empty = bitmaps.sync_term();
    let write = |value| StatusShadowWrite {
        key,
        index: 3,
        value,
    };
    bitmaps.write_shadow(write(7));
    assert_ne!(bitmaps.sync_term(), empty);
    bitmaps.write_shadow(write(0));
    assert_eq!(bitmaps.sync_term(), empty);
    assert_eq!(bitmaps.shadow(key), [0; 4]);

    let mut ports = EventReceivePortState::default();
    ports.construct(crate::EventReceivePort::new(PORT));
    let unset = ports.sync_term();
    ports.set_pending(PORT);
    assert_ne!(ports.sync_term(), unset);

    let mut paused = PausedThreadState::default();
    paused.pause(PausedThread {
        thread: 1,
        decrementer_wakes: true,
    });
    let asleep = paused.sync_term();
    paused.wake(1);
    assert_ne!(paused.sync_term(), asleep);
}

/// The list holds every nonzero lane of a shadow of thread 7 of the
/// logical PPE 5 with 0xAB in doubleword 2, at the fixed lane source 147.
#[test]
fn status_shadow_sync_term_lane_vector_golden() {
    let mut bitmaps = IrqStateBitmapState::default();
    bitmaps.write_shadow(StatusShadowWrite {
        key: IrqStateBitmapKey {
            ppe_id: 5,
            thread: 7,
        },
        index: 2,
        value: 0xAB,
    });
    let lane = |field, value| contribution(LaneIndex::new(147, 0, field, 0), value);
    let sum = [lane(0, 1), lane(1, 5), lane(2, 7), lane(5, 0xAB)]
        .into_iter()
        .fold(0u128, u128::wrapping_add);
    assert_eq!(bitmaps.sync_term(), sum);
}

/// The list holds every nonzero lane of a pending event on port 3, at the
/// fixed lane source 146, and of a wake of thread 1, at the fixed lane
/// source 148.
#[test]
fn pending_event_and_wake_sync_term_lane_vector_golden() {
    let mut ports = EventReceivePortState::default();
    ports.set_pending(3);
    let lane = |source, field, value| contribution(LaneIndex::new(source, 0, field, 0), value);
    assert_eq!(
        ports.sync_term(),
        lane(146, 0, 1).wrapping_add(lane(146, 1, 3))
    );
    let mut paused = PausedThreadState::default();
    paused.wake(1);
    assert_eq!(
        paused.sync_term(),
        lane(148, 0, 1).wrapping_add(lane(148, 1, 1))
    );
}
