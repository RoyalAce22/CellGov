use crate::answer::{HypercallDisposition, HypercallRefusal};
use crate::identity::SYNTHETIC_CONSOLE_IDENTITY;
use crate::request::HypercallRequest;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{
    CLOSE_DEVICE, NET_CONTROL, NET_SET_INTERRUPT_MASK, NET_SET_INTERRUPT_STATUS_INDICATOR,
    OPEN_DEVICE,
};
use cellgov_ps3_abi::lv1::network::{
    INTERRUPT_MASK_NO_BUS_R3, NET_CONTROL_NO_DEVICE_R3, STATUS_INDICATOR_BUS_RANGE_R3,
    STATUS_INDICATOR_NO_DEVICE_R3, STATUS_INDICATOR_SET_R3,
};
use cellgov_time::GuestTicks;

/// `LV1_WRONG_STATE` (-11), zero-extended, as each handler builds it for
/// a state word other than 2: [OFW-4.93 lv1 a:0x308300-0x308308] for 191,
/// [OFW-4.93 lv1 a:0x3080D4-0x3080DC] for 193 and
/// [OFW-4.93 lv1 a:0x307E38-0x307E40] for `net_control`.
const WRONG_STATE: u64 = 0x0000_0000_FFFF_FFF5;

/// The indicator address of the kernel's 191 call, as the 191 tests use.
const INDICATOR: u64 = 0x0087_B3D0;

/// The mask of the kernel's first 193 call
/// [OFW-4.93 lv2_kernel a:0x80000000000A5668-0x80000000000A5680].
const MASK: u64 = 0x1002_0000_0000_6000;

/// The call `number` with `registers` as r3 onward.
fn call(number: u64, registers: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..registers.len()].copy_from_slice(registers);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc: 0x8000_0000_000A_357C,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(40_000),
    }
}

/// The r3 and r4 to r10 of the answer to `number` with `registers`.
fn answer(hv: &mut Hypervisor, number: u64, registers: &[u64]) -> (u64, [u64; 7]) {
    match hv.dispatch(&call(number, registers)) {
        HypercallDisposition::Answered(answer) => (
            answer.r3,
            answer.outputs().expect("an answer with r4 to r10"),
        ),
        HypercallDisposition::Refused(refusal) => {
            panic!("hypercall {number} {registers:x?}: {refusal:?}")
        }
    }
}

fn ok(hv: &mut Hypervisor, number: u64, registers: &[u64]) {
    assert_eq!(answer(hv, number, registers).0, 0, "hypercall {number}");
}

/// A 4.93 hypervisor whose partition opened the network device, set its
/// indicator and mask, and closed it.
fn opened_and_closed() -> Hypervisor {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    ok(&mut hv, OPEN_DEVICE, &[1, 0, 0]);
    ok(
        &mut hv,
        NET_SET_INTERRUPT_STATUS_INDICATOR,
        &[1, 0, INDICATOR, 0],
    );
    ok(&mut hv, NET_SET_INTERRUPT_MASK, &[1, 0, MASK, 0]);
    ok(&mut hv, CLOSE_DEVICE, &[1, 0]);
    assert!(hv.groups().network().torn_down());
    hv
}

/// The state word check comes before r6, the address and the indicator
/// check of 191 [OFW-4.93 lv1 a:0x3082F4-0x308308], before r6 of 193
/// [OFW-4.93 lv1 a:0x3080C8-0x3080DC], and before the request code of
/// `net_control` [OFW-4.93 lv1 a:0x307E2C-0x307E40]. Each exit returns
/// r31 [OFW-4.93 lv1 a:0x3083A8] [OFW-4.93 lv1 a:0x30817C]
/// [OFW-4.93 lv1 a:0x307EE0], and the vector's entry zeroes the return
/// slots [OFW-4.93 lv1 a:0x0C08].
#[test]
fn after_a_close_each_network_call_gets_wrong_state_with_r4_to_r10_cleared_and_no_change() {
    let mut hv = opened_and_closed();
    let closed = hv.clone();
    let calls: [(u64, &[u64]); 10] = [
        (NET_SET_INTERRUPT_STATUS_INDICATOR, &[1, 0, INDICATOR, 0]),
        (
            NET_SET_INTERRUPT_STATUS_INDICATOR,
            &[1, 0, INDICATOR + 1, 1],
        ),
        (NET_SET_INTERRUPT_STATUS_INDICATOR, &[1, 0, 1 << 48, 0]),
        (NET_SET_INTERRUPT_MASK, &[1, 0, MASK, 0]),
        (NET_SET_INTERRUPT_MASK, &[1, 0, MASK, 1]),
        (NET_CONTROL, &[1, 0, 1, 0, 0, 0]),
        (NET_CONTROL, &[1, 0, 4, 1, 0, 0]),
        (NET_CONTROL, &[1, 0, 0, 0, 0, 0]),
        (NET_CONTROL, &[1, 0, 2, 0, 0, 0]),
        (NET_CONTROL, &[1, 0, 12, 0, 0, 0]),
    ];
    for (number, registers) in calls {
        assert_eq!(
            answer(&mut hv, number, registers),
            (WRONG_STATE, [0; 7]),
            "hypercall {number} {registers:x?}"
        );
        assert_eq!(hv, closed, "hypercall {number} {registers:x?}");
    }
}

/// The bus and device checks come before the state word
/// [OFW-4.93 lv1 a:0x30829C-0x3082B0] [OFW-4.93 lv1 a:0x308070-0x308084]
/// [OFW-4.93 lv1 a:0x307DD4-0x307DE8].
#[test]
fn after_a_close_the_bus_and_device_checks_still_come_first() {
    let mut hv = opened_and_closed();
    let calls: [(u64, &[u64], u64); 4] = [
        (
            NET_SET_INTERRUPT_STATUS_INDICATOR,
            &[5, 0, INDICATOR, 0],
            STATUS_INDICATOR_BUS_RANGE_R3,
        ),
        (
            NET_SET_INTERRUPT_STATUS_INDICATOR,
            &[1, 1, INDICATOR, 0],
            STATUS_INDICATOR_NO_DEVICE_R3,
        ),
        (
            NET_SET_INTERRUPT_MASK,
            &[2, 0, MASK, 0],
            INTERRUPT_MASK_NO_BUS_R3,
        ),
        (NET_CONTROL, &[1, 1, 1, 0, 0, 0], NET_CONTROL_NO_DEVICE_R3),
    ];
    for (number, registers, r3) in calls {
        assert_eq!(
            answer(&mut hv, number, registers),
            (r3, [0; 7]),
            "hypercall {number} {registers:x?}"
        );
    }
}

/// The reopen's setup stores 2 in the state word and leaves the mask 0
/// and the indicator -1 that the teardown stored
/// [OFW-4.93 lv1 a:0x23E174-0x23E1F8] [OFW-4.93 lv1 a:0x23E048-0x23E060].
#[test]
fn after_a_close_and_a_reopen_each_network_call_works_again_with_no_indicator_and_mask_0() {
    let mut hv = opened_and_closed();
    ok(&mut hv, OPEN_DEVICE, &[1, 0, 0]);
    let network = hv.groups().network();
    assert!(!network.torn_down());
    assert_eq!(network.status_indicator(), None);
    assert_eq!(network.interrupt_mask(), Some(0));

    ok(
        &mut hv,
        NET_SET_INTERRUPT_STATUS_INDICATOR,
        &[1, 0, INDICATOR, 0],
    );
    assert_eq!(hv.groups().network().status_indicator(), Some(INDICATOR));
    assert_eq!(
        answer(
            &mut hv,
            NET_SET_INTERRUPT_STATUS_INDICATOR,
            &[1, 0, INDICATOR, 0]
        ),
        (STATUS_INDICATOR_SET_R3, [0; 7])
    );
    ok(&mut hv, NET_SET_INTERRUPT_MASK, &[1, 0, 0xC000_0000, 0]);
    assert_eq!(hv.groups().network().interrupt_mask(), Some(0xC000_0000));
    let mac = SYNTHETIC_CONSOLE_IDENTITY.mac_address;
    assert_eq!(
        answer(&mut hv, NET_CONTROL, &[1, 0, 1, 0, 0, 0]),
        (0, [mac, 0, 0, 0, 0, 0, 0])
    );
}

/// No source gives the state word before the first open.
#[test]
fn before_the_first_open_each_network_call_stays_a_refusal() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for (number, registers) in [
        (
            NET_SET_INTERRUPT_STATUS_INDICATOR,
            &[1, 0, INDICATOR, 0][..],
        ),
        (NET_SET_INTERRUPT_MASK, &[1, 0, MASK, 0][..]),
        (NET_CONTROL, &[1, 0, 1, 0, 0, 0][..]),
    ] {
        let disposition = hv.dispatch(&call(number, registers));
        assert!(
            matches!(
                &disposition,
                HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput { what, .. })
                    if what.contains("before the partition opened the network device")
            ),
            "hypercall {number}: {disposition:?}"
        );
    }
}
