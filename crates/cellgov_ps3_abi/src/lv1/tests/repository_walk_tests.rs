use super::device_1_tests::capture_keys;
use super::*;

/// The key `bus.<bus>.type` of the partition `lpar`.
fn bus_type(lpar: u64, bus: u64) -> RepositoryKey {
    RepositoryKey {
        lpar,
        n1: first_field(b"bus", bus),
        n2: field(b"type", 0),
        n3: 0,
        n4: 0,
    }
}

#[test]
fn a_walk_covers_only_the_bus_keys_of_its_partition_from_its_first_to_its_last_bus() {
    let walk = RepositoryWalk {
        capture: BUS_CAPTURE,
        lpar: LPAR_ID_PME,
        first_bus: 1,
        last_bus: 2,
    };
    let cases = [
        (bus_type(LPAR_ID_PME, 0), false),
        (bus_type(LPAR_ID_PME, 1), true),
        (bus_type(LPAR_ID_PME, 2), true),
        (bus_type(LPAR_ID_PME, 3), false),
        (bus_type(LPAR_ID_GAME_OS, 1), false),
        (
            RepositoryKey {
                n1: first_field(b"bi", 1),
                ..bus_type(LPAR_ID_PME, 1)
            },
            false,
        ),
    ];
    assert_eq!(cases.len(), 6);
    for (key, covered) in cases {
        assert_eq!(walk.covers(&key), covered, "{key}");
    }
}

/// The capture holds the `type` node of each bus that it keeps, so the
/// buses with a captured `type` node are the buses that the walk covers.
#[test]
fn the_bus_walk_covers_every_captured_node_and_each_bus_whose_type_the_capture_holds() {
    let mut walked = capture_keys("node.tsv");
    walked.extend(capture_keys("unread_node.tsv"));
    assert!(walked.len() > 30, "{} walked rows", walked.len());
    for key in &walked {
        assert!(BUS_WALK.covers(key), "{key} is outside the walk");
    }
    let typed: Vec<u64> = (0..16)
        .filter(|bus| walked.contains(&bus_type(LPAR_ID_PME, *bus)))
        .collect();
    let covered: Vec<u64> = (BUS_WALK.first_bus..=BUS_WALK.last_bus).collect();
    assert_eq!(typed, covered);
}
