use cellgov_ps3_abi::lv1::repository::{
    field, first_field, NodeSource, ReadSite, RepositoryKey, RepositoryNode, SecondValue,
};

use super::*;

const PUP: &str = "0101010101010101010101010101010101010101010101010101010101010101";

fn rgntotal() -> RepositoryKey {
    RepositoryKey {
        lpar: 2,
        n1: first_field(b"bi", 0),
        n2: field(b"rgntotal", 0),
        n3: 0,
        n4: 0,
    }
}

fn captured_node() -> RepositoryNode {
    RepositoryNode {
        key: rgntotal(),
        v1: 0x0f9a_0000,
        v2: SecondValue::Unobserved {
            discarded_by: &[ReadSite { sc: 0x40, lr: 0x44 }],
        },
        firmware: &["4.93"],
        source: NodeSource::ConsoleCapture {
            capture: "tests/micro/x",
            address: 0x80,
        },
    }
}

fn pup_row() -> PupRow {
    PupRow {
        pup_sha256: PUP.to_string(),
        fw: "4.93".to_string(),
        size_bytes: 1,
        image_version: "0x0000000000000000".to_string(),
        source_note: "test".to_string(),
        acquired: None,
    }
}

fn read_row() -> RepositoryReadRow {
    RepositoryReadRow {
        pup_sha256: PUP.to_string(),
        key: rgntotal(),
        node: rgntotal().path(),
        first: 0,
        answered: true,
    }
}

#[test]
fn an_unobserved_second_value_renders_as_the_unobserved_label_and_never_as_0() {
    let rows = repository_rows(&[captured_node()], &[read_row()], &[pup_row()]).unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].v2, None);
    let text = repository_tsv(&rows).unwrap();
    let parsed = table::parse(&REPOSITORY, &text).unwrap();
    assert_eq!(parsed.rows.len(), 1);
    let v2 = REPOSITORY
        .columns
        .iter()
        .position(|c| c.name == "v2")
        .expect("a v2 column");
    assert_eq!(parsed.rows[0][v2], REPOSITORY_UNOBSERVED);
    assert_ne!(parsed.rows[0][v2], hex(0));
}

#[test]
fn the_v2_column_refuses_a_label_other_than_unobserved() {
    let rows = repository_rows(&[captured_node()], &[read_row()], &[pup_row()]).unwrap();
    let text = repository_tsv(&rows).unwrap();
    let other = text.replace(REPOSITORY_UNOBSERVED, "unknown");
    assert!(matches!(
        table::parse(&REPOSITORY, &other),
        Err(ArchiveError::BadCell { column: "v2", .. })
    ));
}

#[test]
fn a_logged_refusal_of_a_site_gated_node_names_the_site_and_not_only_a_stale_log() {
    let refused = RepositoryReadRow {
        answered: false,
        ..read_row()
    };
    assert_eq!(
        repository_rows(&[captured_node()], &[refused], &[pup_row()]),
        Err(RepositoryTableError::RefusedAtUnlistedSite {
            pup_sha256: PUP.to_string(),
            key: rgntotal(),
        })
    );
}

fn boot_read(values: Option<(u64, u64)>) -> cellgov_lv1::RepositoryRead {
    cellgov_lv1::RepositoryRead {
        key: rgntotal(),
        values,
        unit: cellgov_event::UnitId::new(1),
        pc: 0x40,
    }
}

#[test]
fn a_key_answered_at_one_site_and_refused_at_another_is_refused_and_never_logged_as_answered() {
    let reads = [boot_read(Some((0x0f9a_0000, 0))), boot_read(None)];
    assert_eq!(
        repository_read_rows_of(PUP, &reads),
        Err(MixedRepositoryRead {
            key: rgntotal(),
            answered_at: 0,
            refused_at: 1,
        })
    );
}
