use cellgov_event::UnitId;
use cellgov_lv1::RepositoryRead;
use cellgov_ps3_abi::lv1::repository::{
    field, first_field, NodeSource, RepositoryKey, RepositoryNode, SecondValue,
};

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

fn bi(second: &[u8], index: u64) -> RepositoryKey {
    RepositoryKey {
        lpar: 2,
        n1: first_field(b"bi", 0),
        n2: field(second, index),
        n3: 0,
        n4: 0,
    }
}

const DUMP: NodeSource = NodeSource::PsdevwikiDump {
    alias: "PS3DEVWIKI-REPO",
    section: "3.41_GameOS",
};

fn node(key: RepositoryKey, v1: u64) -> RepositoryNode {
    node_at(key, v1, &["1.50"])
}

fn node_at(key: RepositoryKey, v1: u64, firmware: &'static [&'static str]) -> RepositoryNode {
    RepositoryNode {
        key,
        v1,
        v2: SecondValue::Given(0),
        firmware,
        source: DUMP,
    }
}

fn read(key: RepositoryKey, values: Option<(u64, u64)>) -> RepositoryRead {
    RepositoryRead {
        key,
        values,
        unit: UnitId::new(1),
        pc: 0x40,
    }
}

fn read_row(n: u8, key: RepositoryKey, first: usize, answered: bool) -> RepositoryReadRow {
    RepositoryReadRow {
        pup_sha256: pup(n),
        key,
        node: key.path(),
        first,
        answered,
    }
}

#[test]
fn one_boot_gives_one_row_per_key_at_its_first_read() {
    let reads = [
        read(bi(b"pun", 0), Some((1, 0))),
        read(bi(b"pu", 0), Some((1, 0))),
        read(bi(b"pun", 0), Some((1, 0))),
        read(bi(b"pu", 1), None),
    ];
    let rows = repository_read_rows_of(&pup(1), &reads).unwrap();
    assert_eq!(
        rows,
        vec![
            read_row(1, bi(b"pu", 0), 1, true),
            read_row(1, bi(b"pu", 1), 3, false),
            read_row(1, bi(b"pun", 0), 0, true),
        ]
    );
}

#[test]
fn a_node_row_lists_the_firmware_the_node_names() {
    let pups = [pup_row(1, "1.50"), pup_row(2, "4.93")];
    let reads = [
        read_row(2, bi(b"pun", 0), 0, true),
        read_row(1, bi(b"pun", 0), 0, true),
        read_row(1, bi(b"pu", 1), 1, false),
    ];
    let rows = repository_rows(
        &[node_at(bi(b"pun", 0), 1, &["1.50", "4.93"])],
        &[],
        &reads,
        &pups,
    )
    .unwrap();
    assert_eq!(
        rows,
        vec![RepositoryRow {
            key: bi(b"pun", 0),
            node: "bi.pun".to_string(),
            v1: 1,
            v2: Some(0),
            fw: vec!["1.50".to_string(), "4.93".to_string()],
            source: "psdevwiki",
            citation: "PS3DEVWIKI-REPO",
            section: Some("3.41_GameOS".to_string()),
        }]
    );
}

#[test]
fn two_nodes_of_one_key_give_two_rows_ascending_by_firmware() {
    let pups = [pup_row(1, "1.50"), pup_row(2, "4.93")];
    let reads = [
        read_row(1, bi(b"pun", 0), 0, true),
        read_row(2, bi(b"pun", 0), 0, true),
    ];
    let nodes = [
        node_at(bi(b"pun", 0), 2, &["4.93"]),
        node_at(bi(b"pun", 0), 1, &["1.50"]),
    ];
    let rows = repository_rows(&nodes, &[], &reads, &pups).unwrap();
    let cells: Vec<(u64, Vec<String>)> = rows.into_iter().map(|r| (r.v1, r.fw)).collect();
    assert_eq!(
        cells,
        vec![(1, vec!["1.50".to_string()]), (2, vec!["4.93".to_string()])]
    );
}

#[test]
fn a_node_no_boot_reads_is_refused() {
    let pups = [pup_row(1, "1.50")];
    let reads = [read_row(1, bi(b"pun", 0), 0, true)];
    let nodes = [node(bi(b"pun", 0), 1), node(bi(b"spun", 0), 6)];
    assert_eq!(
        repository_rows(&nodes, &[], &reads, &pups),
        Err(RepositoryTableError::UnreadNode {
            key: bi(b"spun", 0),
            fw: "1.50".to_string(),
        })
    );
}

#[test]
fn a_node_that_names_a_firmware_whose_boot_does_not_read_it_is_refused() {
    let pups = [pup_row(1, "1.50"), pup_row(2, "4.93")];
    let reads = [
        read_row(1, bi(b"pun", 0), 0, true),
        read_row(2, bi(b"pu", 0), 0, false),
    ];
    let nodes = [node_at(bi(b"pun", 0), 1, &["1.50", "4.93"])];
    assert_eq!(
        repository_rows(&nodes, &[], &reads, &pups),
        Err(RepositoryTableError::UnreadNode {
            key: bi(b"pun", 0),
            fw: "4.93".to_string(),
        })
    );
}

#[test]
fn a_read_answered_at_a_firmware_the_node_does_not_name_is_stale() {
    let pups = [pup_row(1, "1.50"), pup_row(2, "4.93")];
    let reads = [
        read_row(1, bi(b"pun", 0), 0, true),
        read_row(2, bi(b"pun", 0), 0, true),
    ];
    assert_eq!(
        repository_rows(&[node(bi(b"pun", 0), 1)], &[], &reads, &pups),
        Err(RepositoryTableError::StaleRead {
            pup_sha256: pup(2),
            key: bi(b"pun", 0),
            answered: true,
        })
    );
}

#[test]
fn a_logged_state_the_node_table_contradicts_is_stale() {
    let pups = [pup_row(1, "1.50")];
    let answered_without_row = [read_row(1, bi(b"pu", 0), 0, true)];
    assert_eq!(
        repository_rows(&[], &[], &answered_without_row, &pups),
        Err(RepositoryTableError::StaleRead {
            pup_sha256: pup(1),
            key: bi(b"pu", 0),
            answered: true,
        })
    );
    let refused_with_row = [read_row(1, bi(b"pu", 0), 0, false)];
    assert_eq!(
        repository_rows(&[node(bi(b"pu", 0), 1)], &[], &refused_with_row, &pups),
        Err(RepositoryTableError::StaleRead {
            pup_sha256: pup(1),
            key: bi(b"pu", 0),
            answered: false,
        })
    );
}

#[test]
fn a_read_of_an_unknown_pup_is_refused() {
    let reads = [read_row(9, bi(b"pun", 0), 0, true)];
    assert_eq!(
        repository_rows(
            &[node(bi(b"pun", 0), 1)],
            &[],
            &reads,
            &[pup_row(1, "1.50")]
        ),
        Err(RepositoryTableError::UnknownPup { pup_sha256: pup(9) })
    );
}

#[test]
fn bi_pu_index_0_and_index_1_are_two_read_rows_that_round_trip() {
    let rows = vec![
        read_row(1, bi(b"pu", 1), 3, false),
        read_row(1, bi(b"pu", 0), 1, true),
    ];
    let text = repository_read_tsv(&rows).unwrap();
    let parsed = repository_read_rows(&archive::parse(&REPOSITORY_READ, &text).unwrap()).unwrap();
    assert_eq!(parsed.len(), 2);
    assert_eq!(parsed[0], rows[1]);
    assert_eq!(parsed[1], rows[0]);
    assert_eq!(parsed[0].node, "bi.pu");
    assert_eq!(parsed[1].node, "bi.pu+1");
}

#[test]
fn a_key_word_renders_as_16_hex_digits() {
    let rows = vec![read_row(1, bi(b"pu", 1), 0, false)];
    let text = repository_read_tsv(&rows).unwrap();
    let line = text.lines().last().unwrap();
    let cells: Vec<&str> = line.split('\t').collect();
    assert_eq!(
        &cells[1..6],
        &[
            "2",
            "0x0000000062690000",
            "0x7075000000000001",
            "0x0000000000000000",
            "0x0000000000000000"
        ]
    );
}

#[test]
fn replacing_a_pups_reads_keeps_every_other_pup() {
    let mut rows = vec![
        read_row(1, bi(b"pun", 0), 0, true),
        read_row(2, bi(b"pun", 0), 0, true),
    ];
    replace_repository_reads(&mut rows, &pup(1), vec![read_row(1, bi(b"pu", 0), 0, true)]);
    assert_eq!(
        rows,
        vec![
            read_row(2, bi(b"pun", 0), 0, true),
            read_row(1, bi(b"pu", 0), 0, true),
        ]
    );
}
