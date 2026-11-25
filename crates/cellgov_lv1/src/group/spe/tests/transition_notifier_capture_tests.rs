//! The hypervisor answers the 4.93 kernel's boot call of hypercall 64
//! after its construct of the boot SPE and its port. The spe id and the
//! port id are the values that the reference console's kernel holds, read
//! from the committed capture `tests/micro/lv2_logical_spe_object`.

use super::*;
use crate::{answered_numbers, Hypervisor};
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{
    CONSTRUCT_EVENT_RECEIVE_PORT, CONSTRUCT_LOGICAL_SPE, CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
};
use cellgov_ps3_abi::lv1::spe::LOGICAL_SPE_OBJECT_CAPTURE;
use cellgov_time::GuestTicks;

/// The value of the field `field` in the capture. Both reads of the word
/// must agree.
fn captured(field: &str) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(LOGICAL_SPE_OBJECT_CAPTURE)
        .join("peek/cech20-cex-493/spe_object.tsv");
    let name = path.display().to_string();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{name}: {e}"));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("address\tfield\twidth\treads\tagree\tvalue"),
        "{name}: header"
    );
    let row: Vec<&str> = lines
        .map(|line| line.split('\t').collect::<Vec<_>>())
        .find(|row| row.get(1) == Some(&field))
        .unwrap_or_else(|| panic!("{name}: no row for {field}"));
    assert_eq!((row[3], row[4]), ("2", "yes"), "{name}: {field} reads");
    u64::from_str_radix(row[5].trim_start_matches("0x"), 16)
        .unwrap_or_else(|e| panic!("{name}: {field}: {e}"))
}

/// A thread 0 call of hypercall `number` at the `sc` address `pc`.
fn call(number: u64, pc: u64, inputs: [u64; 8]) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// The kernel's boot calls before hypercall 64 that make the SPE and the
/// port: the address space, then the construct
/// [OFW-4.93 lv2_kernel a:0x800000000004f188] and the port
/// [OFW-4.93 lv2_kernel a:0x800000000004f218].
fn boot_setup() -> [HypercallRequest; 3] {
    [
        call(
            CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
            0,
            [0, 2, 0x1014_0000_0000_0000, 0, 0, 0, 0, 0],
        ),
        call(
            CONSTRUCT_LOGICAL_SPE,
            0x8000_0000_0004_f188,
            [12, 12, 12, 12, 12, 2, 1, 0],
        ),
        call(CONSTRUCT_EVENT_RECEIVE_PORT, 0x8000_0000_0004_f218, [0; 8]),
    ]
}

/// The kernel's call with the captured spe id in r3, the mask 1 in r4 and
/// the captured port id in r5
/// [OFW-4.93 lv2_kernel a:0x800000000004f228-0x800000000004f238].
fn boot_call() -> HypercallRequest {
    call(
        SET_SPE_TRANSITION_NOTIFIER,
        0x8000_0000_0004_f238,
        [captured("spe_id"), 1, captured("port"), 0, 0, 0, 0, 0],
    )
}

#[test]
fn the_hypervisor_answers_the_boot_call_and_keeps_the_captured_notifier() {
    assert!(answered_numbers().contains(&SET_SPE_TRANSITION_NOTIFIER));
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    for setup in boot_setup() {
        assert!(
            matches!(hv.dispatch(&setup), HypercallDisposition::Answered(_)),
            "hypercall {}: {:?}",
            setup.number,
            hv.refusal()
        );
    }
    let before = hv.sync_term();
    let HypercallDisposition::Answered(answer) = hv.dispatch(&boot_call()) else {
        panic!("{:?}", hv.refusal());
    };
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs(), Some([0; 7]));
    let expected = TransitionNotifier {
        spe: captured("spe_id"),
        port: captured("port"),
        mask: 1,
    };
    let held: Vec<TransitionNotifier> = hv
        .groups()
        .spe()
        .notifiers()
        .notifiers()
        .values()
        .copied()
        .collect();
    assert_eq!(held, [expected]);
    assert_ne!(hv.sync_term(), before);
    assert_eq!(hv.refusal(), None);
}
