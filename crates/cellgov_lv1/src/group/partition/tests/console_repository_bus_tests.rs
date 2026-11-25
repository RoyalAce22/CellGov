//! The hypervisor answers the 4.93 kernel's bus reads with the nodes that
//! lv1 holds on the reference console, read from the committed capture
//! `tests/micro/lv1_repository_bus`.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{field, first_field, RepositoryKey};
use cellgov_time::GuestTicks;

use crate::{HypercallDisposition, HypercallRefusal, HypercallRequest, Hypervisor};

const CAPTURE: &str = "tests/micro/lv1_repository_bus/peek/cech20-cex-493";

/// The `sc` of the 4.93 kernel's repository read wrapper
/// [OFW-4.93 lv2_kernel a:0x80000000000922fc].
const WRAPPER_SC: u64 = 0x8000_0000_0009_22fc;

/// The data rows of the capture's `node.tsv`, each split into its cells.
fn captured_nodes() -> Vec<Vec<String>> {
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
    lines
        .map(|line| line.split('\t').map(str::to_string).collect())
        .collect()
}

fn hex(cell: &str) -> u64 {
    let digits = cell
        .strip_prefix("0x")
        .unwrap_or_else(|| panic!("{cell} is not 0x hex"));
    u64::from_str_radix(digits, 16).unwrap_or_else(|e| panic!("{cell}: {e}"))
}

/// A read of lpar 1's key `n` through the kernel's read wrapper, which
/// sets r3 = 1 [OFW-4.93 lv2_kernel a:0x80000000000922d4].
fn read(n: [u64; 4], lr: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: 91,
        inputs: [1, n[0], n[1], n[2], n[3], 0, 0, 0],
        pc: WRAPPER_SC,
        lr,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

fn dispatch_4_93(request: &HypercallRequest) -> HypercallDisposition {
    let mut hv = Hypervisor::new();
    hv.set_firmware("4.93");
    hv.dispatch(request)
}

/// The bus search reads `bus.0.type` first
/// [OFW-4.93 lv2_kernel a:0x8000000000277804-0x800000000027780c]. lv1
/// returns status 0 with v1 and v2 when it finds the node
/// [OFW-4.93 lv1 a:0x2E53AC-0x2E53B8].
#[test]
fn the_bus_0_type_read_answers_status_0_and_the_captured_node_values() {
    let n = [first_field(b"bus", 0), field(b"type", 0), 0, 0];
    let rows = captured_nodes();
    let row = rows
        .iter()
        .find(|row| [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])] == n)
        .expect("a captured row for bus.0.type");
    assert_eq!((row[0].as_str(), row[1].as_str()), ("4.93", "1"));
    let answer = match dispatch_4_93(&read(n, 0x8000_0000_0009_61e4)) {
        HypercallDisposition::Answered(answer) => answer,
        HypercallDisposition::Refused(refusal) => panic!("{refusal}"),
    };
    assert_eq!(answer.r3, 0, "r3");
    let outputs = answer.outputs().expect("r4 to r10");
    assert_eq!(outputs[..2], [hex(&row[9]), hex(&row[10])], "r4 and r5");
}

#[test]
fn every_captured_bus_node_answers_its_captured_values_at_4_93() {
    let rows = captured_nodes();
    assert!(!rows.is_empty());
    for row in &rows {
        let n = [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])];
        let answer = match dispatch_4_93(&read(n, 0)) {
            HypercallDisposition::Answered(answer) => answer,
            HypercallDisposition::Refused(refusal) => panic!("{refusal}"),
        };
        let outputs = answer.outputs().expect("r4 to r10");
        assert_eq!(
            (answer.r3, outputs[0], outputs[1]),
            (0, hex(&row[9]), hex(&row[10])),
            "{row:?}"
        );
    }
}

/// The device loop reads the type of device 1 after the handler of device
/// 0 returns [OFW-4.93 lv2_kernel a:0x80000000000937ec-0x8000000000093808].
/// No logged boot reaches that read, so the node table does not serve the
/// node.
#[test]
fn the_unserved_bus_1_device_1_type_read_is_a_refusal_that_names_its_key() {
    let n = [
        first_field(b"bus", 1),
        field(b"dev", 1),
        field(b"type", 0),
        0,
    ];
    let request = read(n, 0x8000_0000_0009_5f84);
    let HypercallDisposition::Refused(refusal) = dispatch_4_93(&request) else {
        panic!("bus.1.dev.1.type is answered");
    };
    assert_eq!(
        refusal,
        HypercallRefusal::RepositoryNode {
            key: RepositoryKey {
                lpar: 1,
                n1: n[0],
                n2: n[1],
                n3: n[2],
                n4: n[3],
            },
            unit: UnitId::new(0),
            pc: WRAPPER_SC,
        }
    );
}
