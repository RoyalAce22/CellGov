//! The hypervisor answers the 4.93 kernel's `ss.param.update.status` read
//! with the node that lv1 holds on the reference console, read from the
//! committed capture `tests/micro/lv1_repository_list`.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{field, first_field, LIST_CAPTURE};
use cellgov_time::GuestTicks;

use crate::{HypercallDisposition, HypercallRequest, Hypervisor};

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The row of the capture's `node.tsv` whose key words are `key`, split
/// into its cells.
fn captured_node(key: [u64; 4]) -> Vec<String> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(LIST_CAPTURE)
        .join("node.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2"),
        "{}",
        path.display()
    );
    let rows: Vec<Vec<String>> = lines
        .map(|line| line.split('\t').map(str::to_string).collect::<Vec<_>>())
        .filter(|row: &Vec<String>| {
            row.len() == 11 && [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])] == key
        })
        .collect();
    assert_eq!(rows.len(), 1, "{}: one row of the key", path.display());
    let row = rows.into_iter().next().expect("one row");
    assert_eq!((row[7].as_str(), row[8].as_str()), ("2", "yes"));
    row
}

/// The kernel's reader puts lpar 1 in r3 and the four key words in r4 to
/// r7 before its `sc`
/// [OFW-4.93 lv2_kernel a:0x8000000000050ab4-0x8000000000050b24]. The
/// constructor that the 4.93 boot runs calls it from
/// [OFW-4.93 lv2_kernel a:0x80000000000a1f84]. lv1 returns status 0 with
/// v1 and v2 when it finds the node [OFW-4.93 lv1 a:0x2E53AC-0x2E53B8].
#[test]
fn the_update_status_read_answers_status_0_and_the_captured_node_values() {
    let key = [
        first_field(b"ss", 0),
        field(b"param", 0),
        field(b"update", 0),
        field(b"status", 0),
    ];
    let row = captured_node(key);
    let inputs = [1, key[0], key[1], key[2], key[3], 0, 0, 0];
    assert_eq!(row[1].parse::<u64>().expect("a decimal lpar"), inputs[0]);
    let request = HypercallRequest {
        unit: UnitId::new(0),
        number: 91,
        inputs,
        pc: 0x8000_0000_0005_0b24,
        lr: 0x8000_0000_000a_1f88,
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
}
