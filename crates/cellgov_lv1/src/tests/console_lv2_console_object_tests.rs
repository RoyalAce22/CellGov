//! The hypervisor answers the 4.93 kernel's boot-time console calls on
//! the console id that the reference console's kernel holds, read from
//! the committed capture `tests/micro/lv2_console_object`.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::call::{LOGICAL_CONSOLE_CONSTRUCT, LOGICAL_CONSOLE_DESTRUCT};
use cellgov_time::GuestTicks;

use crate::{HypercallDisposition, HypercallRequest, Hypervisor};

const CAPTURE: &str = "lv2_console_object/peek/cech20-cex-493/console_object.tsv";

/// The kernel's `sc` for hypercall 105 [OFW-4.93 lv2_kernel a:0x800000000000fc54].
const CONSTRUCT_PC: u64 = 0x8000_0000_0000_fc54;

/// The kernel's `sc` for hypercall 106 [OFW-4.93 lv2_kernel a:0x80000000000104e8].
const DESTRUCT_PC: u64 = 0x8000_0000_0001_04e8;

/// The value of the field `field` in the capture. Both reads of the
/// field must agree.
fn captured(field: &str) -> u64 {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(CAPTURE);
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

/// The kernel's call of hypercall `number` on console `console`. The
/// construct passes the inputs that the kernel's boot construct sets
/// [OFW-4.93 lv2_kernel a:0x80000000000109ac-0x80000000000109d0].
fn kernel_call(number: u64, console: u64) -> HypercallRequest {
    let (inputs, pc) = if number == LOGICAL_CONSOLE_CONSTRUCT {
        ([console, 0, 0, 0xFF0, 0xFF0, 0, 0, 0], CONSTRUCT_PC)
    } else {
        ([console, 0, 0, 0, 0, 0, 0, 0], DESTRUCT_PC)
    };
    HypercallRequest {
        unit: UnitId::new(1),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(0),
    }
}

#[test]
fn the_kernels_construct_destruct_construct_on_the_captured_id_answers_0_at_each_call() {
    let console = captured("id");
    assert_eq!(
        console,
        captured("id_base") + 1,
        "the id the kernel derives"
    );
    assert_eq!(
        (captured("state"), captured("flag")),
        (1, 0),
        "the console finished the sequence"
    );
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    let sequence = [
        LOGICAL_CONSOLE_CONSTRUCT,
        LOGICAL_CONSOLE_DESTRUCT,
        LOGICAL_CONSOLE_CONSTRUCT,
    ];
    for (step, number) in sequence.into_iter().enumerate() {
        let d = hv.dispatch(&kernel_call(number, console));
        assert!(
            matches!(&d, HypercallDisposition::Answered(a) if a.r3 == 0),
            "step {step}, hypercall {number}: {d:?}"
        );
    }
    assert_eq!(hv.refusal(), None);
}
