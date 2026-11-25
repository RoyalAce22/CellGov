use super::*;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_time::GuestTicks;

fn request(bus: u64, device: u64, r5: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[bus, device, r5]);
    HypercallRequest {
        unit: UnitId::new(0),
        number: OPEN_DEVICE,
        inputs,
        pc: 0x8000_0000_000A_E110,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(33_909),
    }
}

/// Bus 4 device ids above 6 other than -1, from each side of the 32-bit
/// and sign boundaries.
const ABSENT_DEVICES: [u64; 6] = [
    7,
    8,
    0xFFFF_FFFF,
    0x1_0000_0000,
    0x8000_0000_0000_0000,
    u64::MAX - 1,
];

/// The device bus states that the answer must not read: empty, the
/// storage bus held, and device 0 of bus 1 open.
fn states() -> [DeviceBusState; 3] {
    let mut held = DeviceBusState::default();
    held.apply(DeviceBusChange::StorageBusOpened(StorageBusOpen { r5: 0 }));
    let mut opened = DeviceBusState::default();
    opened.apply(DeviceBusChange::Opened(OpenDevice { bus: 1, device: 0 }));
    [DeviceBusState::default(), held, opened]
}

/// [OFW-4.93 lv1 a:0x237780-0x23778C] builds -6 with r4 marked, and the
/// open function returns it past its zero-extension
/// [OFW-4.93 lv1 a:0x237790] [OFW-4.93 lv1 a:0x237634]; the vector's
/// entry zeroes r4 to r10 [OFW-4.93 lv1 a:0x0C08].
#[test]
fn a_bus_4_device_above_6_other_than_minus_1_gets_no_device_and_no_change() {
    let expected =
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(0x0000_0002_FFFF_FFFA, [0; 7]));
    let states = states();
    assert_eq!(states.len(), 3);
    for (index, state) in states.iter().enumerate() {
        for device in ABSENT_DEVICES {
            for r5 in [0, u64::MAX] {
                assert_eq!(
                    open_device(&request(4, device, r5), Some("4.93"), state),
                    expected,
                    "state {index} device {device:#x} r5 {r5:#x}"
                );
            }
        }
    }
}

#[test]
fn an_absent_device_open_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let firmwares = [None, Some("3.70"), Some("4.92")];
    assert_eq!(firmwares.len(), 3);
    for firmware in firmwares {
        let call = request(4, 7, 0);
        assert_eq!(
            open_device(&call, firmware, &DeviceBusState::default()),
            UNREAD_FIRMWARE.refusal(&call),
            "firmware {firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_answers_an_absent_bus_4_device_and_keeps_its_state() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let HypercallDisposition::Answered(storage) = hv.dispatch(&request(4, u64::MAX, 0)) else {
        panic!("the storage bus open is refused");
    };
    assert_eq!(storage.r3, 0, "storage bus open r3");
    let before = hv.clone();
    let term = hv.sync_term();
    for device in ABSENT_DEVICES {
        let HypercallDisposition::Answered(answer) = hv.dispatch(&request(4, device, 0)) else {
            panic!("device {device:#x} is refused");
        };
        assert_eq!(answer.r3, 0x0000_0002_FFFF_FFFA, "device {device:#x} r3");
        assert_eq!(
            answer.outputs(),
            Some([0; 7]),
            "device {device:#x} r4 to r10"
        );
    }
    assert_eq!(hv, before);
    assert_eq!(hv.sync_term(), term);
}
