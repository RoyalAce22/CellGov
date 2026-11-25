use super::*;
use crate::lv1::status::{zero_extended, LV1_NO_ENTRY};

/// The key of lpar 1's device 1 of bus 1, with the last two key words `n`.
fn device_1(n: [u64; 2]) -> RepositoryKey {
    RepositoryKey {
        lpar: LPAR_ID_PME,
        n1: first_field(b"bus", 1),
        n2: field(b"dev", 1),
        n3: n[0],
        n4: n[1],
    }
}

/// The served v1 and v2 of the 4.93 node `key`, and the lv1 address the
/// capture read them at.
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

/// The keys of every row of the bus capture's file `name`.
fn capture_keys(name: &str) -> Vec<RepositoryKey> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(BUS_CAPTURE)
        .join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let word = |cell: &str| {
        u64::from_str_radix(cell.trim_start_matches("0x"), 16)
            .unwrap_or_else(|e| panic!("{cell}: {e}"))
    };
    text.lines()
        .skip(1)
        .map(|line| {
            let cells: Vec<&str> = line.split('\t').collect();
            RepositoryKey {
                lpar: cells[1].parse().unwrap_or_else(|e| panic!("{line}: {e}")),
                n1: word(cells[2]),
                n2: word(cells[3]),
                n3: word(cells[4]),
                n4: word(cells[5]),
            }
        })
        .collect()
}

#[test]
fn the_regions_and_interrupts_of_device_1_answer_the_captured_values() {
    let cases = [
        (
            device_1([field(b"reg", 0), field(b"type", 0)]),
            (3, 0, 0x0068_a578),
        ),
        (
            device_1([field(b"reg", 0), field(b"data", 0)]),
            (0x0301_0000, 0x0001_0000, 0x0068_a5f8),
        ),
        (
            device_1([field(b"reg", 1), field(b"type", 0)]),
            (4, 0, 0x0068_a678),
        ),
        (
            device_1([field(b"reg", 1), field(b"data", 0)]),
            (0x0381_0000, 0x0001_0000, 0x0068_a6f8),
        ),
        (device_1([field(b"intr", 0), 0]), (3, 0x10, 0x0068_a478)),
        (device_1([field(b"intr", 1), 0]), (4, 0x0a, 0x0068_a4f8)),
    ];
    assert_eq!(cases.len(), 6);
    for (key, (v1, v2, address)) in cases {
        assert_eq!(
            served(&key),
            (v1, SecondValue::Given(v2), bus_capture(address)),
            "{key}"
        );
    }
}

/// [OFW-4.93 lv1 a:0x2E3E9C-0x2E3EB8] `li r3, 30; sldi r3, r3, 32;
/// oris r3, r3, 0xffff; ori r3, r3, 0xfffa`.
#[test]
fn the_no_node_r3_is_no_entry_with_0x1e_in_the_high_word() {
    assert_eq!(NO_NODE_R3, (0x1E << 32) | zero_extended(LV1_NO_ENTRY));
}

/// The key of lpar 1's device 2 of bus 1, with the last two key words `n`.
fn device_2(n: [u64; 2]) -> RepositoryKey {
    RepositoryKey {
        n2: field(b"dev", 2),
        ..device_1(n)
    }
}

#[test]
fn the_regions_and_interrupts_of_device_2_answer_the_captured_values() {
    let cases = [
        (
            device_2([field(b"reg", 0), field(b"type", 0)]),
            (3, 0, 0x0068_a978),
        ),
        (
            device_2([field(b"reg", 0), field(b"data", 0)]),
            (0x0302_0000, 0x0001_0000, 0x0068_a9f8),
        ),
        (
            device_2([field(b"reg", 1), field(b"type", 0)]),
            (4, 0, 0x0068_aa78),
        ),
        (
            device_2([field(b"reg", 1), field(b"data", 0)]),
            (0x0382_0000, 0x0001_0000, 0x0068_aaf8),
        ),
        (device_2([field(b"intr", 0), 0]), (3, 0x11, 0x0068_a878)),
        (device_2([field(b"intr", 1), 0]), (4, 0x0b, 0x0068_a8f8)),
    ];
    assert_eq!(cases.len(), 6);
    for (key, (v1, v2, address)) in cases {
        assert_eq!(
            served(&key),
            (v1, SecondValue::Given(v2), bus_capture(address)),
            "{key}"
        );
    }
}

/// The key of lpar 1's device 3 of bus 1, with the last two key words `n`.
fn device_3(n: [u64; 2]) -> RepositoryKey {
    RepositoryKey {
        n2: field(b"dev", 3),
        ..device_1(n)
    }
}

/// The handler of type 6 reads the id and interrupt 0 of device 3
/// [OFW-4.93 lv2_kernel a:0x8000000000093318-0x80000000000933ac].
#[test]
fn device_3_of_bus_1_answers_its_type_id_and_interrupt_0_from_the_lv1_capture() {
    let cases = [
        (device_3([field(b"type", 0), 0]), (6, 0, 0x0068_abf8)),
        (device_3([field(b"id", 0), 0]), (3, 0, 0x0068_ab78)),
        (device_3([field(b"intr", 0), 0]), (5, 0x2f, 0x0068_ac78)),
    ];
    assert_eq!(cases.len(), 3);
    for (key, (v1, v2, address)) in cases {
        assert_eq!(
            served(&key),
            (v1, SecondValue::Given(v2), bus_capture(address)),
            "{key}"
        );
    }
}

#[test]
fn the_absent_keys_are_the_loop_ends_of_devices_1_and_2_and_region_0_of_device_3_at_4_93() {
    let keys: Vec<_> = ABSENT_NODES.iter().map(|absent| absent.key).collect();
    assert_eq!(
        keys,
        [
            device_1([field(b"intr", 2), 0]),
            device_1([field(b"reg", 2), field(b"type", 0)]),
            device_2([field(b"intr", 2), 0]),
            device_2([field(b"reg", 2), field(b"type", 0)]),
            device_3([field(b"reg", 0), field(b"type", 0)]),
        ]
    );
    for absent in ABSENT_NODES {
        assert_eq!(absent.firmware, ["4.93"], "{}", absent.key);
        assert_eq!(absent.capture, BUS_CAPTURE, "{}", absent.key);
    }
}

/// The walk kept every node of buses 0 to 3 in `node.tsv` or
/// `unread_node.tsv`, so an absent key of those buses is in neither file.
#[test]
fn no_absent_key_is_a_node_of_the_bus_capture_or_a_served_node() {
    let mut walked = capture_keys("node.tsv");
    walked.extend(capture_keys("unread_node.tsv"));
    assert!(walked.len() > 30, "{} walked rows", walked.len());
    assert_eq!(ABSENT_NODES.len(), 5);
    for absent in ABSENT_NODES {
        assert!(!walked.contains(&absent.key), "{} is walked", absent.key);
        for fw in absent.firmware {
            assert_eq!(lookup(&absent.key, fw), None, "{} is served", absent.key);
        }
    }
}

#[test]
fn lookup_absent_matches_only_a_listed_key_at_a_listed_firmware() {
    let region_2 = device_1([field(b"reg", 2), field(b"type", 0)]);
    assert_eq!(
        lookup_absent(&region_2, "4.93").map(|absent| absent.key),
        Some(region_2)
    );
    assert_eq!(lookup_absent(&region_2, "3.70"), None);
    let cases = [
        device_1([field(b"reg", 2), field(b"data", 0)]),
        device_1([field(b"reg", 3), field(b"type", 0)]),
        device_1([field(b"intr", 3), 0]),
        RepositoryKey {
            lpar: 2,
            ..region_2
        },
    ];
    assert_eq!(cases.len(), 4);
    for key in cases {
        assert_eq!(lookup_absent(&key, "4.93"), None, "{key}");
    }
}

/// After device 1, the device loop reads the type of device 2, and the
/// handler of a later device of type 4 reads its id
/// [OFW-4.93 lv2_kernel a:0x8000000000093484-0x8000000000093490].
#[test]
fn device_2_of_bus_1_answers_type_4_and_id_2_from_the_lv1_capture() {
    assert_eq!(
        served(&device_2([field(b"type", 0), 0])),
        (4, SecondValue::Given(0), bus_capture(0x0068_a7f8))
    );
    assert_eq!(
        served(&device_2([field(b"id", 0), 0])),
        (2, SecondValue::Given(0), bus_capture(0x0068_a778))
    );
}
