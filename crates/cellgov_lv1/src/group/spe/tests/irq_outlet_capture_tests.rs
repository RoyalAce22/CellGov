//! The hypervisor answers the 4.93 kernel's three calls for its boot SPE
//! with the outlet ids that the reference console's kernel holds. The ids
//! come from the committed capture `tests/micro/lv2_logical_spe_object`.

use super::*;
use crate::group::spe::LogicalSpe;
use cellgov_event::UnitId;
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

/// The kernel's call for the class `class`
/// [OFW-4.93 lv2_kernel a:0x800000000004f1f4], with the captured spe id
/// in r3 and the class in r4
/// [OFW-4.93 lv2_kernel a:0x800000000004f1e8-0x800000000004f1ec].
fn boot_call(class: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: GET_SPE_IRQ_OUTLET,
        inputs: [captured("spe_id"), class, 0, 0, 0, 0, 0, 0],
        pc: 0x8000_0000_0004_f1f4,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(0),
    }
}

#[test]
fn the_kernels_boot_calls_get_the_captured_outlet_ids() {
    let mut spes = LogicalSpeState::default();
    spes.construct(LogicalSpe {
        id: captured("spe_id"),
        spe_type: 1,
        vas_id: 2,
        area: 0,
        shadow: captured("shadow"),
        shadow_real: 0,
    });
    for (class, field) in (0u64..).zip(["outlet_0", "outlet_1", "outlet_2"]) {
        let d = irq_outlet(&boot_call(class), Some("4.93"), &spes);
        let HypercallDisposition::Answered(answer) = d else {
            panic!("{field}: {d:?}");
        };
        assert_eq!(answer.r3, 0, "{field}: r3");
        assert_eq!(
            answer.outputs().map(|o| o[0]),
            Some(captured(field)),
            "{field}: r4"
        );
    }
}
