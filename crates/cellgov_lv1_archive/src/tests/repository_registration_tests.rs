use cellgov_ps3_abi::lv1::repository::{field, first_field, RepositoryKey};

use super::*;

fn pup(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn pup_row(n: u8, fw: &str) -> PupRow {
    PupRow {
        pup_sha256: pup(n),
        fw: fw.to_string(),
        size_bytes: 1,
        image_version: "0x0000000000000000".to_string(),
        source_note: "test".to_string(),
        acquired: None,
    }
}

/// The key `bus.4.dev.<device>.<name>` of lpar 1.
fn device_key(device: u64, name: &[u8]) -> RepositoryKey {
    RepositoryKey {
        lpar: 1,
        n1: first_field(b"bus", 4),
        n2: field(b"dev", device),
        n3: field(name, 0),
        n4: 0,
    }
}

fn read_row(n: u8, key: RepositoryKey, answered: bool) -> RepositoryReadRow {
    RepositoryReadRow {
        pup_sha256: pup(n),
        key,
        node: key.path(),
        first: 0,
        answered,
    }
}

/// v1 and v2 of the 4.93 row of `key` in `node.tsv` of the registration
/// capture.
fn captured(key: RepositoryKey) -> (u64, u64) {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(REGISTRATION_CAPTURE)
        .join("node.tsv");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("fw\tlpar\tn1\tn2\tn3\tn4\taddress\treads\tagree\tv1\tv2"),
        "{}: header",
        path.display()
    );
    let hex = |cell: &str| {
        u64::from_str_radix(cell.trim_start_matches("0x"), 16)
            .unwrap_or_else(|e| panic!("{cell}: {e}"))
    };
    let found: Vec<(u64, u64)> = lines
        .map(|line| line.split('\t').collect::<Vec<&str>>())
        .filter(|cells| {
            cells[0] == "4.93"
                && cells[1].parse::<u64>() == Ok(key.lpar)
                && [hex(cells[2]), hex(cells[3]), hex(cells[4]), hex(cells[5])]
                    == [key.n1, key.n2, key.n3, key.n4]
        })
        .map(|cells| (hex(cells[9]), hex(cells[10])))
        .collect();
    assert_eq!(found.len(), 1, "{key} in {}", path.display());
    found[0]
}

#[test]
fn an_answered_read_of_a_registration_node_gets_a_row_with_the_registration_values() {
    let key = device_key(0, b"blk_size");
    let (v1, v2) = captured(key);
    let rows = repository_rows(&[], &[], &[read_row(1, key, true)], &[pup_row(1, "4.93")]);
    assert_eq!(
        rows,
        Ok(vec![RepositoryRow {
            key,
            node: key.path(),
            v1,
            v2: Some(v2),
            fw: vec!["4.93".to_string()],
            source: "console",
            citation: REGISTRATION_CAPTURE,
            section: None,
        }])
    );
}

/// Device 5 is not in the bus table, so its `type` keeps the
/// initializer's placeholder in `node.tsv` of the registration capture.
#[test]
fn an_answered_read_of_a_placeholder_node_gets_the_placeholder_values() {
    let key = device_key(5, b"type");
    let (v1, v2) = captured(key);
    let rows = repository_rows(&[], &[], &[read_row(1, key, true)], &[pup_row(1, "4.93")])
        .expect("the placeholder is a registration node");
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0].v1, rows[0].v2), (v1, Some(v2)));
}

#[test]
fn an_answered_read_of_a_bus_4_key_that_no_registration_writes_is_a_stale_read() {
    let key = device_key(0, b"color");
    assert_eq!(
        repository_rows(&[], &[], &[read_row(1, key, true)], &[pup_row(1, "4.93")]),
        Err(RepositoryTableError::StaleRead {
            pup_sha256: pup(1),
            key,
            answered: true,
        })
    );
}

#[test]
fn an_answered_read_of_a_registration_node_at_a_firmware_with_no_registration_is_a_stale_read() {
    let key = device_key(0, b"blk_size");
    assert_eq!(
        repository_rows(&[], &[], &[read_row(1, key, true)], &[pup_row(1, "3.70")]),
        Err(RepositoryTableError::StaleRead {
            pup_sha256: pup(1),
            key,
            answered: true,
        })
    );
}

#[test]
fn a_refused_read_of_a_registration_node_needs_no_row() {
    let key = device_key(0, b"blk_size");
    assert_eq!(
        repository_rows(&[], &[], &[read_row(1, key, false)], &[pup_row(1, "4.93")]),
        Ok(Vec::new())
    );
}
