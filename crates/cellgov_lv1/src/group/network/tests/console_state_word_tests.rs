//! The network device's state word in the model, held to the console
//! capture of the word while the game OS partition holds the device open.

use crate::answer::HypercallDisposition;
use crate::request::HypercallRequest;
use crate::Hypervisor;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{CLOSE_DEVICE, NET_CONTROL, OPEN_DEVICE};
use cellgov_ps3_abi::lv1::network::{
    NET_CONTROL_CAPTURE, NET_CONTROL_WRONG_STATE_R3, TORN_DOWN_STATE_WORD,
};
use cellgov_ps3_abi::lv1::status::{zero_extended, LV1_SUCCESS};
use cellgov_time::GuestTicks;

/// The state word that the check of `net_control` passes
/// [OFW-4.93 lv1 a:0x307E30-0x307E34].
const PASSING_STATE: u32 = 2;

/// The state word at +8 of the network device's object that the capture
/// read after the boot to the XMB. Panics unless both reads of the word
/// agree.
fn captured_state_word() -> u32 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(NET_CONTROL_CAPTURE)
        .join("peek/cech20-cex-493/net_control.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let rows: Vec<Vec<&str>> = text
        .lines()
        .map(|line| line.split('\t').collect())
        .filter(|row: &Vec<&str>| row.get(1) == Some(&"device_state"))
        .collect();
    assert_eq!(rows.len(), 1, "{name}: device_state rows");
    let row = &rows[0];
    assert_eq!(row.len(), 6, "{name}: {row:?}");
    assert_eq!(
        (row[2], row[3], row[4]),
        ("4", "2", "yes"),
        "{name}: {row:?}"
    );
    u32::from_str_radix(row[5].trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("{name}: {row:?}: {e}"))
}

/// The call `number` with `registers` as r3 onward.
fn call(number: u64, registers: &[u64]) -> HypercallRequest {
    let mut inputs = [0; 8];
    inputs[..registers.len()].copy_from_slice(registers);
    HypercallRequest {
        unit: UnitId::new(0),
        number,
        inputs,
        pc: 0x8000_0000_000A_369C,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(40_000),
    }
}

/// The r3 of the answer to `number` with `registers`.
fn r3(hv: &mut Hypervisor, number: u64, registers: &[u64]) -> u64 {
    match hv.dispatch(&call(number, registers)) {
        HypercallDisposition::Answered(answer) => answer.r3,
        HypercallDisposition::Refused(refusal) => {
            panic!("hypercall {number} {registers:x?}: {refusal:?}")
        }
    }
}

/// Asserts that the model holds the state word `captured` for the network
/// device:
/// - the model holds the teardown's word only when `captured` is that
///   word;
/// - `net_control` passes the state check only when `captured` is the
///   word that the check passes.
fn assert_holds(hv: &mut Hypervisor, captured: u32, after: &str) {
    assert_eq!(
        hv.groups().network().torn_down(),
        captured == TORN_DOWN_STATE_WORD,
        "the teardown's state word after {after}"
    );
    let expected = if captured == PASSING_STATE {
        zero_extended(LV1_SUCCESS)
    } else {
        NET_CONTROL_WRONG_STATE_R3
    };
    assert_eq!(
        r3(hv, NET_CONTROL, &[1, 0, 1, 0, 0, 0]),
        expected,
        "net_control after {after}"
    );
}

/// The kernel's boot open of device 0 of bus 1 is the last call to the
/// device before the capture's read. A close and a reopen run the open's
/// setup again [OFW-4.93 lv1 a:0x23E1C8-0x23E1D4].
#[test]
fn an_open_network_device_holds_the_captured_state_word_after_an_open_and_after_a_reopen() {
    let captured = captured_state_word();
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    assert_eq!(r3(&mut hv, OPEN_DEVICE, &[1, 0, 0]), 0, "open_device");
    assert_holds(&mut hv, captured, "the open");
    assert_eq!(r3(&mut hv, CLOSE_DEVICE, &[1, 0]), 0, "close_device");
    assert_eq!(r3(&mut hv, OPEN_DEVICE, &[1, 0, 0]), 0, "open_device again");
    assert_holds(&mut hv, captured, "the reopen");
}
