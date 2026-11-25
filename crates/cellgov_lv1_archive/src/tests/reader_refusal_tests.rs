use super::*;
use cellgov_lv2_archive::{ArchiveError, Column, ColumnKind, Table, TableSpec};

type Read = fn(&Table) -> Result<usize, ArchiveError>;

fn readers() -> [(&'static TableSpec, Read); 4] {
    [
        (&HYPERCALL_SITE, |t| {
            hypercall_site_rows(t).map(|rows| rows.len())
        }),
        (&HYPERCALL_UNRESOLVED, |t| {
            hypercall_unresolved_rows(t).map(|rows| rows.len())
        }),
        (&REPOSITORY_READ, |t| {
            repository_read_rows(t).map(|rows| rows.len())
        }),
        (&HYPERCALL, |t| {
            census_answered(t).map(|numbers| numbers.len())
        }),
    ]
}

fn well_formed_cell(column: &Column) -> String {
    match column.kind {
        ColumnKind::Integer | ColumnKind::IntegerList => "1".to_string(),
        ColumnKind::Ident | ColumnKind::Locator => "a".to_string(),
        ColumnKind::VersionList => "4.93".to_string(),
        ColumnKind::Sha256 => "0".repeat(64),
        ColumnKind::Hex32 => "0x00000000".to_string(),
        ColumnKind::Hex64 => format!("0x{}", "0".repeat(16)),
        ColumnKind::Hex64OrLabel(label) => label.to_string(),
        ColumnKind::Enum(labels) => labels[0].to_string(),
    }
}

fn well_formed_row(spec: &TableSpec) -> Vec<String> {
    spec.columns.iter().map(well_formed_cell).collect()
}

fn one_row(spec: &'static TableSpec, row: Vec<String>) -> Table {
    Table {
        spec,
        rows: vec![row],
    }
}

#[test]
fn every_reader_reads_a_well_formed_row_of_its_spec() {
    let readers = readers();
    assert_eq!(readers.len(), 4);
    for (spec, read) in readers {
        let table = one_row(spec, well_formed_row(spec));
        // `census_answered` counts answered rows; the first state label
        // of the fixture row is `answered`.
        assert_eq!(read(&table), Ok(1), "{}", spec.name);
    }
}

#[test]
fn every_reader_refuses_a_table_of_another_spec() {
    let readers = readers();
    assert_eq!(readers.len(), 4);
    for (spec, read) in readers {
        let other = Table {
            spec: &REPOSITORY,
            rows: Vec::new(),
        };
        let refusal = read(&other);
        assert!(
            matches!(&refusal, Err(ArchiveError::Header { table, .. }) if *table == spec.name),
            "{}: {refusal:?}",
            spec.name
        );
    }
}

#[test]
fn every_reader_refuses_a_row_of_the_wrong_width() {
    let readers = readers();
    assert_eq!(readers.len(), 4);
    for (spec, read) in readers {
        let width = spec.columns.len();
        let mut short = well_formed_row(spec);
        short.pop();
        let mut long = well_formed_row(spec);
        long.push("1".to_string());
        for (row, found) in [(short, width - 1), (long, width + 1)] {
            assert_eq!(
                read(&one_row(spec, row)),
                Err(ArchiveError::CellCount {
                    table: spec.name,
                    line: 2,
                    expected: width,
                    found,
                }),
                "{}: a row of {found} cells",
                spec.name
            );
        }
    }
}

#[test]
fn every_reader_refuses_a_cell_its_column_does_not_take() {
    let readers = readers();
    assert_eq!(readers.len(), 4);
    for (spec, read) in readers {
        let mut row = well_formed_row(spec);
        row[0] = "?".to_string();
        assert_eq!(
            read(&one_row(spec, row)),
            Err(ArchiveError::BadCell {
                table: spec.name,
                line: 2,
                column: spec.columns[0].name,
                cell: "?".to_string(),
                expected: spec.columns[0].kind.describe(),
            }),
            "{}",
            spec.name
        );
    }
}
