//! After lv1 registers each device of the bus table, the hypervisor
//! answers each `bus.4` node of the committed captures
//! `tests/micro/lv1_storage_registration` and
//! `tests/micro/lv1_storage_placeholder_nodes` with the captured values.

use cellgov_ps3_abi::lv1::storage_registration::{registration, REGISTRATION_CAPTURE};
use cellgov_time::GuestTicks;

use super::*;
use crate::group::storage::device_registration::register_device;

/// The console capture of the `bus.4` nodes that [`REGISTRATION_CAPTURE`]
/// holds no value for.
const PLACEHOLDER_CAPTURE: &str = "tests/micro/lv1_storage_placeholder_nodes/peek/cech20-cex-493";

const NODE_HEADER: &str = "fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2";

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// The lpar and key words, and v1 and v2, of each row of the node file
/// `name` of the capture `dir`, after a check that both reads of each
/// node agree.
fn captured_nodes(dir: &str, name: &str) -> Vec<([u64; 5], [u64; 2])> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(dir)
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(NODE_HEADER), "{}", path.display());
    lines
        .map(|line| {
            let row: Vec<&str> = line.split('\t').collect();
            assert_eq!(row.len(), 11, "{line:?}");
            assert_eq!((row[0], row[7], row[8]), ("4.93", "2", "yes"), "{line:?}");
            let lpar: u64 = row[1].parse().unwrap_or_else(|e| panic!("{line:?}: {e}"));
            (
                [lpar, hex(row[2]), hex(row[3]), hex(row[4]), hex(row[5])],
                [hex(row[9]), hex(row[10])],
            )
        })
        .collect()
}

/// A read of the key `key`, the lpar first.
fn read(key: [u64; 5]) -> HypercallRequest {
    let [lpar, n1, n2, n3, n4] = key;
    HypercallRequest {
        unit: UnitId::new(0),
        number: 91,
        inputs: [lpar, n1, n2, n3, n4, 0, 0, 0],
        pc: 0x8000_0000_0000_ce3c,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

#[test]
fn each_captured_node_answers_its_captured_values_after_each_table_device_registers() {
    let source = registration("4.93").expect("a 4.93 registration source");
    let mut groups = crate::group::state::GroupStates::default();
    for device in source.devices {
        let change = register_device(Some("4.93"), device.id).expect("a table device");
        groups.apply(change.into());
    }
    let memory = cellgov_mem::GuestMemory::new(0x1000);
    let nodes: Vec<([u64; 5], [u64; 2])> = captured_nodes(REGISTRATION_CAPTURE, "node.tsv")
        .into_iter()
        .chain(captured_nodes(REGISTRATION_CAPTURE, "region.tsv"))
        .chain(captured_nodes(PLACEHOLDER_CAPTURE, "node.tsv"))
        .collect();
    assert_eq!(nodes.len(), 52 + 84 + 154);
    for (key, values) in nodes {
        assert_eq!(
            crate::handlers::answer(&read(key), Some("4.93"), &groups, &memory),
            HypercallDisposition::Answered(HypercallAnswer::success(values)),
            "{key:x?}"
        );
    }
}
