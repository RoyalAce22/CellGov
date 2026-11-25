use super::*;
use crate::spec::OwnerClass;

static ROWS_T: TableSpec = TableSpec {
    name: "rows_t",
    owner: OwnerClass::Generated,
    columns: &[
        Column {
            name: "id",
            kind: ColumnKind::Integer,
            nullable: false,
            references: None,
        },
        Column {
            name: "tag",
            kind: ColumnKind::Enum(&["a", "b"]),
            nullable: false,
            references: None,
        },
        Column {
            name: "list",
            kind: ColumnKind::IntegerList,
            nullable: true,
            references: None,
        },
        Column {
            name: "word",
            kind: ColumnKind::Hex64,
            nullable: false,
            references: None,
        },
        Column {
            name: "errno",
            kind: ColumnKind::Hex32,
            nullable: true,
            references: None,
        },
    ],
    key: &["id"],
    regenerate: Some("r"),
    gate: "g",
};

static OTHER_T: TableSpec = TableSpec {
    name: "other_t",
    owner: OwnerClass::Generated,
    columns: &[Column {
        name: "id",
        kind: ColumnKind::Integer,
        nullable: false,
        references: None,
    }],
    key: &["id"],
    regenerate: Some("r"),
    gate: "g",
};

fn row(cells: &[&str]) -> Vec<String> {
    cells.iter().map(|cell| (*cell).to_string()).collect()
}

fn table(rows: Vec<Vec<String>>) -> Table {
    Table {
        spec: &ROWS_T,
        rows,
    }
}

const GOOD: [&str; 5] = ["7", "b", "1,5", "0x00000000000000ff", "0x80010002"];

#[test]
fn checked_rows_reads_each_cell_of_a_well_formed_row() {
    let parsed = table(vec![
        row(&GOOD),
        row(&["8", "a", NONE, "0x0000000000000001", NONE]),
    ]);
    let rows = checked_rows(&ROWS_T, &parsed).unwrap();
    assert_eq!(rows.len(), 2);
    let first = rows[0];
    assert_eq!(first.integer::<u64>(0), Ok(7));
    assert_eq!(first.label(1, |l| (l == "b").then_some(2u8)), Ok(2));
    assert_eq!(first.integers(2), Ok(vec![1, 5]));
    assert_eq!(first.hex64(3), Ok(0xff));
    assert_eq!(first.hex32(4), Ok(0x8001_0002));
    assert_eq!(first.optional(4), Ok(Some("0x80010002")));
    let second = rows[1];
    assert_eq!(second.integers(2), Ok(Vec::new()));
    assert_eq!(second.optional(4), Ok(None));
}

#[test]
fn checked_rows_refuses_a_table_of_another_spec() {
    let other = Table {
        spec: &OTHER_T,
        rows: Vec::new(),
    };
    assert_eq!(
        checked_rows(&ROWS_T, &other).unwrap_err(),
        ArchiveError::Header {
            table: "rows_t",
            expected: "id\ttag\tlist\tword\terrno".to_string(),
            found: "id".to_string(),
        }
    );
}

#[test]
fn checked_rows_refuses_a_row_of_the_wrong_width() {
    let short = row(&GOOD[..4]);
    let mut long = row(&GOOD);
    long.push("extra".to_string());
    let cases = [(short, 4), (long, 6)];
    assert_eq!(cases.len(), 2);
    for (bad, found) in cases {
        let parsed = table(vec![row(&GOOD), bad]);
        assert_eq!(
            checked_rows(&ROWS_T, &parsed).unwrap_err(),
            ArchiveError::CellCount {
                table: "rows_t",
                line: 3,
                expected: 5,
                found,
            },
            "a row of {found} cells"
        );
    }
}

#[test]
fn checked_rows_refuses_a_cell_its_column_does_not_take() {
    let mut bad_tag = row(&GOOD);
    bad_tag[1] = "c".to_string();
    let mut null_word = row(&GOOD);
    null_word[3] = NONE.to_string();
    let parsed = table(vec![bad_tag]);
    assert_eq!(
        checked_rows(&ROWS_T, &parsed).unwrap_err(),
        ArchiveError::BadCell {
            table: "rows_t",
            line: 2,
            column: "tag",
            cell: "c".to_string(),
            expected: "one of a, b".to_string(),
        }
    );
    let parsed = table(vec![null_word]);
    assert_eq!(
        checked_rows(&ROWS_T, &parsed).unwrap_err(),
        ArchiveError::NoneRefused {
            table: "rows_t",
            line: 2,
            column: "word",
        }
    );
}

#[test]
fn an_accessor_refuses_a_cell_of_another_kind_and_an_index_past_the_columns() {
    let parsed = table(vec![row(&GOOD)]);
    let rows = checked_rows(&ROWS_T, &parsed).unwrap();
    assert_eq!(rows.len(), 1);
    let only = rows[0];
    let bad = |column: &'static str, cell: &str, kind: ColumnKind| ArchiveError::BadCell {
        table: "rows_t",
        line: 2,
        column,
        cell: cell.to_string(),
        expected: kind.describe(),
    };
    assert_eq!(
        only.integer::<u64>(1),
        Err(bad("tag", "b", ColumnKind::Integer))
    );
    assert_eq!(
        only.integer::<u8>(3),
        Err(bad("word", GOOD[3], ColumnKind::Integer))
    );
    assert_eq!(
        only.integers(1),
        Err(bad("tag", "b", ColumnKind::IntegerList))
    );
    assert_eq!(only.hex64(0), Err(bad("id", "7", ColumnKind::Hex64)));
    assert_eq!(only.hex64(4), Err(bad("errno", GOOD[4], ColumnKind::Hex64)));
    assert_eq!(only.hex32(3), Err(bad("word", GOOD[3], ColumnKind::Hex32)));
    assert_eq!(
        only.label(1, |_| None::<u8>),
        Err(bad("tag", "b", ColumnKind::Enum(&["a", "b"])))
    );
    assert_eq!(
        only.text(5),
        Err(ArchiveError::CellCount {
            table: "rows_t",
            line: 2,
            expected: 5,
            found: 5,
        })
    );
}
