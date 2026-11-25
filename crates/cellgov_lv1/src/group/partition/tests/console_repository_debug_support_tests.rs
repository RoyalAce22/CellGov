//! The hypervisor answers the 4.93 kernel's `ss.param.debug.support`
//! read with the node that lv1 holds on the reference console, read from
//! the committed capture `tests/micro/lv1_repository_debug_support`.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{field, first_field};
use cellgov_time::GuestTicks;

use crate::{HypercallAnswer, HypercallDisposition, HypercallRequest, Hypervisor};

const CAPTURE: &str = "tests/micro/lv1_repository_debug_support/peek/cech20-cex-493";

/// The one data row of the capture's `node.tsv`, split into its cells.
fn captured_node() -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(CAPTURE)
        .join("node.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2"),
        "{}",
        path.display()
    );
    let row: Vec<String> = lines
        .next()
        .unwrap_or_else(|| panic!("{}: no data row", path.display()))
        .split('\t')
        .map(str::to_string)
        .collect();
    assert_eq!(lines.next(), None, "{}: one data row", path.display());
    assert_eq!(row.len(), 11, "{}: {row:?}", path.display());
    assert_eq!((row[7].as_str(), row[8].as_str()), ("2", "yes"));
    row
}

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The kernel builds lpar 1 and these key words in r3 to r7 before its
/// `sc` [OFW-4.93 lv2_kernel a:0x8000000000050a08-0x8000000000050a48];
/// lv1 returns status 0 with v1 and v2 when it finds the node
/// [OFW-4.93 lv1 a:0x2E53AC-0x2E53B8].
#[test]
fn the_debug_support_read_answers_status_0_and_the_captured_node_values() {
    let row = captured_node();
    let inputs = [
        1,
        first_field(b"ss", 0),
        field(b"param", 0),
        field(b"debug", 0),
        field(b"support", 0),
        0,
        0,
        0,
    ];
    assert_eq!(row[1].parse::<u64>().expect("a decimal lpar"), inputs[0]);
    assert_eq!(
        [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])],
        [inputs[1], inputs[2], inputs[3], inputs[4]],
        "the captured key"
    );
    let request = HypercallRequest {
        unit: UnitId::new(0),
        number: 91,
        inputs,
        pc: 0x8000_0000_0005_0a50,
        lr: 0x8000_0000_0005_0aa0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    };
    let mut hv = Hypervisor::new();
    hv.set_firmware(&row[0]);
    let answer = match hv.dispatch(&request) {
        HypercallDisposition::Answered(answer) => answer,
        HypercallDisposition::Refused(refusal) => panic!("{refusal}"),
    };
    assert_eq!(answer.r3, 0, "r3");
    let outputs = answer.outputs().expect("r4 to r10");
    assert_eq!(outputs[..2], [hex(&row[9]), hex(&row[10])], "r4 and r5");
    assert_eq!(
        answer,
        HypercallAnswer::success([hex(&row[9]), hex(&row[10])]),
        "r6 to r10 cleared and no other effect"
    );
}
