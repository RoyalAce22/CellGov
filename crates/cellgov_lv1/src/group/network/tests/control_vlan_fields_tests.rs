use super::*;
use crate::group::device_bus::DeviceBusChange;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::OPEN_DEVICE;
use cellgov_time::GuestTicks;

/// The `sc` of the 4.93 kernel's request 4
/// [OFW-4.93 lv2_kernel a:0x80000000000A2868].
const KERNEL_SC: u64 = 0x8000_0000_000A_2868;

/// The kernel's request 4 for device 0 of bus 1 with `selector` in r6 and
/// 0 in r7 and r8 [OFW-4.93 lv2_kernel a:0x80000000000A2844-0x80000000000A2864].
fn kernel_call(selector: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: NET_CONTROL,
        inputs: [1, 0, 4, selector, 0, 0, 0, 0],
        pc: KERNEL_SC,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(33_289),
    }
}

fn control(r: &HypercallRequest, fields: VlanFields) -> HypercallDisposition {
    let mut devices = DeviceBusState::default();
    devices.apply(DeviceBusChange::Opened(NETWORK_DEVICE));
    net_control(
        r,
        Some("4.93"),
        &devices,
        &SYNTHETIC_CONSOLE_IDENTITY,
        fields,
    )
}

/// [OFW-4.93 lv1 a:0x245014-0x245018] loads -6 and goes to the exit,
/// which stores no output [OFW-4.93 lv1 a:0x244B28-0x244B54].
#[test]
fn every_selector_gets_minus_6_zero_extended_with_the_console_fields() {
    let selectors: Vec<u64> = (0..=21).chain([1 << 32, u64::MAX]).collect();
    assert_eq!(selectors.len(), 24);
    for selector in selectors {
        assert_eq!(
            control(&kernel_call(selector), CONSOLE_VLAN_FIELDS),
            HypercallDisposition::Answered(HypercallAnswer::lv1_status(
                0x0000_0000_FFFF_FFFA,
                [0; 7]
            )),
            "selector {selector:#x}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x244EC8-0x244EE0] with the case at
/// [OFW-4.93 lv1 a:0x244E88-0x244E94]. The capability check keeps bit 63
/// alone, so a doubleword with every other bit set also gives the case.
#[test]
fn selector_5_gets_294_or_296_from_the_flag_byte_when_bit_63_is_clear() {
    let cases = [(0, 294), (1, 296), (0xFF, 296)];
    let capabilities = [0, 0xFFFF_FFFF_FFFF_FFFE];
    assert_eq!(cases.len() * capabilities.len(), 6);
    for capability in capabilities {
        for (flag, id) in cases {
            let fields = VlanFields { capability, flag };
            assert_eq!(
                control(&kernel_call(5), fields),
                HypercallDisposition::Answered(HypercallAnswer::lv1_status(
                    0,
                    [id, 0, 0, 0, 0, 0, 0]
                )),
                "capability {capability:#x} flag {flag:#x}"
            );
        }
    }
}

/// The kernel's first request 4 has the selector 1
/// [OFW-4.93 lv2_kernel a:0x80000000000A384C-0x80000000000A385C].
#[test]
fn the_hypervisor_fails_the_kernels_first_request_4_with_minus_6_without_a_state_change() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let open = HypercallRequest {
        number: OPEN_DEVICE,
        inputs: [1, 0, 0, 0, 0, 0, 0, 0],
        ..kernel_call(0)
    };
    let HypercallDisposition::Answered(opened) = hv.dispatch(&open) else {
        panic!("open_device refused");
    };
    assert_eq!(opened.r3, 0);
    let before = hv.clone();
    let HypercallDisposition::Answered(answer) = hv.dispatch(&kernel_call(1)) else {
        panic!("net_control refused");
    };
    assert_eq!(answer.r3, 0x0000_0000_FFFF_FFFA);
    assert_eq!(answer.outputs(), Some([0; 7]));
    assert_eq!(hv, before);
}
