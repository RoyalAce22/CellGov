use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{field, first_field, RepositoryKey};

fn request(number: u64, pc: u64, inputs: [u64; 8]) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(3),
        number,
        inputs,
        pc,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

fn bi(second: &[u8], index: u64) -> [u64; 8] {
    [
        2,
        first_field(b"bi", 0),
        field(second, index),
        0,
        0,
        0x55,
        0x66,
        8,
    ]
}

/// The repository dump [PS3DEVWIKI-REPO s:3.41_GameOS] gives
/// `bi.pun` = 1 and holds no `bi.pu` at index 1.
#[test]
fn the_log_holds_each_repository_read_in_order_with_its_answer_or_refusal() {
    let mut hv = Hypervisor::new();
    hv.set_firmware("1.50");
    hv.dispatch(&request(91, 0x10, bi(b"pun", 0)));
    hv.dispatch(&request(105, 0x14, [0; 8]));
    hv.dispatch(&request(91, 0x18, bi(b"pu", 1)));
    let key = |second: &[u8], index: u64| RepositoryKey {
        lpar: 2,
        n1: first_field(b"bi", 0),
        n2: field(second, index),
        n3: 0,
        n4: 0,
    };
    assert_eq!(
        hv.repository_reads(),
        &[
            RepositoryRead {
                key: key(b"pun", 0),
                values: Some((1, 0)),
                unit: UnitId::new(3),
                pc: 0x10,
            },
            RepositoryRead {
                key: key(b"pu", 1),
                values: None,
                unit: UnitId::new(3),
                pc: 0x18,
            },
        ]
    );
}

#[test]
fn a_hypercall_other_than_91_adds_no_read() {
    let mut hv = Hypervisor::new();
    hv.dispatch(&request(74, 0x10, bi(b"pun", 0)));
    assert!(hv.repository_reads().is_empty());
}
