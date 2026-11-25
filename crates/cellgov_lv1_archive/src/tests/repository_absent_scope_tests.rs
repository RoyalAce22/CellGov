use cellgov_ps3_abi::lv1::repository::{
    field, first_field, AbsentNode, RepositoryKey, RepositoryWalk, BUS_WALK,
};
use cellgov_ps3_abi::lv1::storage_registration::STORAGE_BUS_INDEX;

use super::*;

fn pup() -> String {
    "01".repeat(32)
}

fn pup_row() -> PupRow {
    PupRow {
        pup_sha256: pup(),
        fw: "4.93".to_string(),
        size_bytes: 1,
        image_version: "0x0000000000000000".to_string(),
        source_note: "test".to_string(),
        acquired: None,
    }
}

/// An answered 4.93 read of `key`, which an absent entry of `key` lets
/// through.
fn answered(key: RepositoryKey) -> RepositoryReadRow {
    RepositoryReadRow {
        pup_sha256: pup(),
        key,
        node: key.path(),
        first: 0,
        answered: true,
    }
}

/// The key `bus.<bus>.dev.1.reg.2.type` of the partition `lpar`.
fn region_2(lpar: u64, bus: u64) -> RepositoryKey {
    RepositoryKey {
        lpar,
        n1: first_field(b"bus", bus),
        n2: field(b"dev", 1),
        n3: field(b"reg", 2),
        n4: field(b"type", 0),
    }
}

#[test]
fn an_absent_key_outside_the_buses_or_the_partition_of_its_walk_is_refused() {
    let keys = [region_2(1, 5), region_2(2, 1)];
    assert_eq!(keys.len(), 2);
    for key in keys {
        let absent = AbsentNode {
            key,
            firmware: &["4.93"],
            walk: BUS_WALK,
        };
        assert_eq!(
            repository_rows(&[], &[absent], &[answered(key)], &[pup_row()]),
            Err(RepositoryTableError::AbsentKeyOutsideWalk {
                key,
                capture: BUS_WALK.capture,
            }),
            "{key}"
        );
    }
}

#[test]
fn an_absent_key_that_the_storage_registration_writes_is_refused() {
    let bus_id = RepositoryKey {
        lpar: 1,
        n1: first_field(b"bus", STORAGE_BUS_INDEX),
        n2: field(b"id", 0),
        n3: 0,
        n4: 0,
    };
    let walk = RepositoryWalk {
        first_bus: STORAGE_BUS_INDEX,
        last_bus: STORAGE_BUS_INDEX,
        ..BUS_WALK
    };
    let absent = AbsentNode {
        key: bus_id,
        firmware: &["4.93"],
        walk,
    };
    assert_eq!(
        repository_rows(&[], &[absent], &[answered(bus_id)], &[pup_row()]),
        Err(RepositoryTableError::RegisteredAbsentKey {
            key: bus_id,
            fw: "4.93".to_string(),
        })
    );
}
