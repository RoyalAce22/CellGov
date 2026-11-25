//! The hypervisor answers the 4.93 kernel's storage bus search with the
//! bus nodes that lv1 holds on the reference console, read from the
//! committed capture `tests/micro/lv1_repository_bus`.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{field, first_field, RepositoryKey};
use cellgov_time::GuestTicks;

use crate::{HypercallDisposition, HypercallRefusal, HypercallRequest, Hypervisor};

const CAPTURE: &str = "tests/micro/lv1_repository_bus/peek/cech20-cex-493";

/// The `sc` of the 4.93 kernel's repository read wrapper
/// [OFW-4.93 lv2_kernel a:0x80000000000922fc].
const WRAPPER_SC: u64 = 0x8000_0000_0009_22fc;

/// The data rows of the capture's `unread_node.tsv`.
fn unread_rows() -> Vec<Vec<String>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(CAPTURE)
        .join("unread_node.tsv");
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

fn bus_field(bus: u64, word: &[u8]) -> [u64; 4] {
    [first_field(b"bus", bus), field(word, 0), 0, 0]
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

/// The id accessor calls the wrapper from
/// [OFW-4.93 lv2_kernel a:0x80000000000962b8], and the type accessor
/// from [OFW-4.93 lv2_kernel a:0x80000000000961e0]. lv1 returns status 0
/// with v1 and v2 when it finds the node
/// [OFW-4.93 lv1 a:0x2E53AC-0x2E53B8].
#[test]
fn the_storage_search_reads_answer_status_0_and_the_unread_node_values() {
    let reads = [
        (bus_field(0, b"id"), 0x8000_0000_0009_62bc),
        (bus_field(2, b"id"), 0x8000_0000_0009_62bc),
        (bus_field(2, b"type"), 0x8000_0000_0009_61e4),
        (bus_field(3, b"id"), 0x8000_0000_0009_62bc),
        (bus_field(3, b"type"), 0x8000_0000_0009_61e4),
    ];
    assert_eq!(reads.len(), 5);
    let rows = unread_rows();
    for (n, lr) in reads {
        let matching: Vec<&Vec<String>> = rows
            .iter()
            .filter(|row| [hex(&row[2]), hex(&row[3]), hex(&row[4]), hex(&row[5])] == n)
            .collect();
        assert_eq!(matching.len(), 1, "{n:x?}");
        let row = matching[0];
        assert_eq!((row[0].as_str(), row[1].as_str()), ("4.93", "1"));
        let answer = match dispatch_4_93(&read(n, lr)) {
            HypercallDisposition::Answered(answer) => answer,
            HypercallDisposition::Refused(refusal) => panic!("{n:x?}: {refusal}"),
        };
        assert_eq!(answer.r3, 0, "r3 of {n:x?}");
        let outputs = answer.outputs().expect("r4 to r10");
        assert_eq!(
            outputs[..2],
            [hex(&row[9]), hex(&row[10])],
            "r4 and r5 of {n:x?}"
        );
    }
}

/// No committed capture gives the values of bus 4, so its id read stays a
/// refusal.
#[test]
fn the_bus_4_id_read_is_refused() {
    let n = bus_field(4, b"id");
    assert_eq!(
        dispatch_4_93(&read(n, 0x8000_0000_0009_62bc)),
        HypercallDisposition::Refused(HypercallRefusal::RepositoryNode {
            key: RepositoryKey {
                lpar: 1,
                n1: n[0],
                n2: n[1],
                n3: n[2],
                n4: n[3],
            },
            unit: UnitId::new(0),
            pc: WRAPPER_SC,
        })
    );
}
