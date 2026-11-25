use super::*;
use crate::answer::HypercallRefusal;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::device_bus::{STORAGE_BUS, STORAGE_BUS_DEVICE};
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's storage bus open
/// [OFW-4.93 lv2_kernel a:0x80000000000ae110].
const KERNEL_SC: u64 = 0x8000_0000_000A_E110;

fn request(bus: u64, device: u64, r5: u64) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..3].copy_from_slice(&[bus, device, r5]);
    HypercallRequest {
        unit: UnitId::new(0),
        number: OPEN_DEVICE,
        inputs,
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(33_909),
    }
}

/// The kernel's call: bus 4, device -1 and r5 = 0
/// [OFW-4.93 lv2_kernel a:0x80000000000ae100-0x80000000000ae110].
fn kernel_call() -> HypercallRequest {
    request(4, u64::MAX, 0)
}

fn held(r5: u64) -> DeviceBusState {
    let mut state = DeviceBusState::default();
    state.apply(DeviceBusChange::StorageBusOpened(StorageBusOpen { r5 }));
    state
}

fn opened_with(r5: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, [0; 7]);
    answer.push_change(DeviceBusChange::StorageBusOpened(StorageBusOpen { r5 }));
    HypercallDisposition::Answered(answer)
}

#[test]
fn the_kernel_call_names_the_storage_bus_object() {
    assert_eq!(kernel_call().r(3), STORAGE_BUS);
    assert_eq!(kernel_call().r(4), STORAGE_BUS_DEVICE);
}

#[test]
fn a_first_open_of_the_storage_bus_answers_0_and_keeps_r5() {
    let r5s = [0, 1, 0x8000_0000_0000_0000];
    assert_eq!(r5s.len(), 3);
    for r5 in r5s {
        assert_eq!(
            open_device(
                &request(4, u64::MAX, r5),
                Some("4.93"),
                &DeviceBusState::default()
            ),
            opened_with(r5),
            "r5 {r5:#x}"
        );
    }
}

#[test]
fn an_open_of_the_held_storage_bus_gets_busy_and_no_change() {
    let busy =
        HypercallDisposition::Answered(HypercallAnswer::lv1_status(0x0000_0000_FFFF_FFF7, [0; 7]));
    for r5 in [0, 1] {
        assert_eq!(
            open_device(&request(4, u64::MAX, r5), Some("4.93"), &held(0)),
            busy,
            "r5 {r5:#x}"
        );
    }
}

/// The bus 4 lookup reads devices 0 to 6 from the storage bus object's
/// device table [OFW-4.93 lv1 a:0x2365E4-0x2365EC]
/// [OFW-4.93 lv1 a:0x236604-0x23661C].
#[test]
fn device_minus_1_of_another_bus_and_bus_4_table_devices_stay_refusals() {
    let what = "an open of a device of buses 1 to 3 other than devices 0, 1 and 2 of bus 1, or \
                of devices 0 to 6 of bus 4, which lv1 finds through state that CellGov does not \
                model";
    let cases = [(1, u64::MAX), (2, u64::MAX), (3, u64::MAX), (4, 6), (4, 0)];
    assert_eq!(cases.len(), 5);
    for (bus, device) in cases {
        let call = request(bus, device, 0);
        let refusal = HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: OPEN_DEVICE,
            unit: call.unit,
            pc: call.pc,
            what,
        });
        assert_eq!(
            open_device(&call, Some("4.93"), &held(0)),
            refusal,
            "bus {bus} device {device:#x}"
        );
    }
}

#[test]
fn a_storage_bus_open_at_a_firmware_whose_handler_is_not_read_is_a_refusal() {
    let firmwares = [None, Some("3.70"), Some("4.92")];
    assert_eq!(firmwares.len(), 3);
    for firmware in firmwares {
        let call = kernel_call();
        assert_eq!(
            open_device(&call, firmware, &DeviceBusState::default()),
            UNREAD_FIRMWARE.refusal(&call),
            "firmware {firmware:?}"
        );
    }
}

#[test]
fn the_hypervisor_opens_the_storage_bus_once_then_gets_busy() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let empty = hv.sync_term();
    let HypercallDisposition::Answered(first) = hv.dispatch(&kernel_call()) else {
        panic!("the kernel's storage bus open is refused");
    };
    assert_eq!(first.r3, 0, "r3");
    assert_eq!(first.outputs(), Some([0; 7]), "r4 to r10");
    let bus = hv.groups().device_bus();
    assert_eq!(bus.storage_bus_open(), Some(StorageBusOpen { r5: 0 }));
    assert_eq!(bus.open_devices().count(), 0);
    assert_ne!(hv.sync_term(), empty);
    let after_first = hv.clone();
    let HypercallDisposition::Answered(second) = hv.dispatch(&kernel_call()) else {
        panic!("the second storage bus open is refused");
    };
    assert_eq!(second.r3, 0x0000_0000_FFFF_FFF7, "r3");
    assert_eq!(second.outputs(), Some([0; 7]), "r4 to r10");
    assert_eq!(hv, after_first);
}
