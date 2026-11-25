use super::*;
use crate::lv1::partition::LPAR_ID_PME;

/// The key of lpar 1's device `device` of bus 1, with the last two key
/// words `n`.
fn device(device: u64, n: [u64; 2]) -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_PME,
        n1: first_field(b"bus", 1),
        n2: field(b"dev", device),
        n3: n[0],
        n4: n[1],
    }
}

/// The served v1, v2 and lv1 address of the 4.93 node `key`.
fn served(key: &RepositoryKey) -> (u64, SecondValue, NodeSource) {
    let node = lookup(key, "4.93").unwrap_or_else(|| panic!("no 4.93 row for {key}"));
    (node.v1, node.v2, node.source)
}

fn bus_capture(address: u64) -> NodeSource {
    NodeSource::Lv1NodeCapture {
        capture: BUS_CAPTURE,
        address,
    }
}

/// The census names these words: "bus" with index 1, "dev" with index 0,
/// then "intr" with index 0.
#[test]
fn the_served_interrupt_key_is_the_words_the_4_93_census_reads() {
    let key = device(0, [field(b"intr", 0), 0]);
    assert_eq!(
        (key.lpar, key.n1, key.n2, key.n3, key.n4),
        (
            1,
            0x0000_0000_6275_7301,
            0x6465_7600_0000_0000,
            0x696e_7472_0000_0000,
            0
        )
    );
    assert!(lookup(&key, "4.93").is_some());
}

/// The capture's `node.tsv` holds v1 = 2 and v2 = 0 in its row of address
/// 0x68A2F8. The handler of type 3 keeps v2 only when v1 is 2
/// [OFW-4.93 lv2_kernel a:0x8000000000093754-0x8000000000093790].
#[test]
fn interrupt_0_of_device_0_answers_v1_2_and_v2_0_from_the_lv1_capture() {
    assert_eq!(
        served(&device(0, [field(b"intr", 0), 0])),
        (2, SecondValue::Given(0), bus_capture(0x0068_a2f8))
    );
}

/// The capture's `node.tsv` holds the type 4 in its row of address
/// 0x68A3F8, and the id 1 in its row of address 0x68A378.
#[test]
fn device_1_of_bus_1_answers_type_4_and_id_1_from_the_lv1_capture() {
    assert_eq!(
        served(&device(1, [field(b"type", 0), 0])),
        (4, SecondValue::Given(0), bus_capture(0x0068_a3f8))
    );
    assert_eq!(
        served(&device(1, [field(b"id", 0), 0])),
        (1, SecondValue::Given(0), bus_capture(0x0068_a378))
    );
}

/// The handler of a device of type 4 reads `reg.0.type` of the device
/// after its open [OFW-4.93 lv2_kernel a:0x80000000000934d4-0x80000000000934ec].
/// No logged boot reaches that read.
#[test]
fn the_region_0_type_of_device_1_has_no_served_node() {
    assert!(lookup(&device(1, [field(b"reg", 0), field(b"type", 0)]), "4.93").is_none());
}

#[test]
fn a_floor_read_of_a_device_node_is_refused() {
    for fw in ["1.50", "1.94", "3.70"] {
        assert!(
            lookup(&device(0, [field(b"intr", 0), 0]), fw).is_none(),
            "{fw}"
        );
        assert!(
            lookup(&device(1, [field(b"type", 0), 0]), fw).is_none(),
            "{fw}"
        );
        assert!(
            lookup(&device(1, [field(b"id", 0), 0]), fw).is_none(),
            "{fw}"
        );
    }
}
