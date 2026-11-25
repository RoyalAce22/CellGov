use super::*;
use crate::answer::HypercallRefusal;
use crate::group::device_bus::DeviceBusChange;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::network::STATUS_INDICATOR_HANDLERS;
use cellgov_time::GuestTicks;

/// An indicator address in the real mode region, at a multiple of
/// [`STATUS_INDICATOR_ALIGN`].
const INDICATOR: u64 = 0x0001_0000;

/// The call for the network device with the indicator [`INDICATOR`] and
/// r6 = 0, at the `sc` of the 4.93 kernel's call
/// [OFW-4.93 lv2_kernel a:0x80000000000A34F4].
fn request() -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..4].copy_from_slice(&[STATUS_INDICATOR_BUS, STATUS_INDICATOR_DEVICE, INDICATOR, 0]);
    HypercallRequest {
        unit: UnitId::new(0),
        number: NET_SET_INTERRUPT_STATUS_INDICATOR,
        inputs,
        pc: 0x8000_0000_000A_34F4,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(32_835),
    }
}

/// The 4.93 answer to `r` after an open of the network device, with
/// `real_mode_size` as the source of the real mode size.
fn set_from(r: &HypercallRequest, real_mode_size: fn(&str) -> Option<u64>) -> HypercallDisposition {
    let mut devices = DeviceBusState::default();
    devices.apply(DeviceBusChange::Opened(NETWORK_DEVICE));
    set_status_indicator_with(
        r,
        Some("4.93"),
        &devices,
        &NetworkState::default(),
        real_mode_size,
    )
}

/// While this holds, the status indicator handler does not reach its
/// refusal of a firmware whose real mode size no source gives.
#[test]
fn every_firmware_with_a_status_indicator_handler_row_has_a_real_mode_size() {
    let firmwares: Vec<&str> = STATUS_INDICATOR_HANDLERS
        .iter()
        .map(|row| row.firmware)
        .collect();
    assert!(!firmwares.is_empty(), "no status indicator handler row");
    let unsourced: Vec<&str> = firmwares
        .into_iter()
        .filter(|fw| real_mode_size(fw).is_none())
        .collect();
    assert_eq!(unsourced, Vec::<&str>::new());
}

#[test]
fn a_real_mode_indicator_is_set_with_the_repository_real_mode_size() {
    let mut answer = HypercallAnswer::lv1_status(0, []);
    answer.push_change(NetworkChange::StatusIndicatorSet(INDICATOR));
    let set = HypercallDisposition::Answered(answer);
    assert_eq!(set_from(&request(), real_mode_size), set);
    // The shipped entry point passes the repository source.
    let mut devices = DeviceBusState::default();
    devices.apply(DeviceBusChange::Opened(NETWORK_DEVICE));
    assert_eq!(
        set_status_indicator(&request(), Some("4.93"), &devices, &NetworkState::default()),
        set
    );
}

#[test]
fn a_real_mode_indicator_is_refused_when_no_source_gives_the_real_mode_size() {
    let r = request();
    assert_eq!(
        set_from(&r, |_: &str| None),
        HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
            number: NET_SET_INTERRUPT_STATUS_INDICATOR,
            unit: r.unit,
            pc: r.pc,
            what: "a net_set_interrupt_status_indicator at a firmware whose real mode size no \
                   source gives",
        })
    );
}
