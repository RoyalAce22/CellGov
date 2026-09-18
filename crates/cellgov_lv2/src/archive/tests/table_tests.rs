use super::*;
use crate::archive::spec::OwnerClass;

static T: TableSpec = TableSpec {
    name: "t",
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
            name: "arm",
            kind: ColumnKind::Ident,
            nullable: true,
            references: Some(("arm", "arm")),
        },
        Column {
            name: "list",
            kind: ColumnKind::IntegerList,
            nullable: true,
            references: None,
        },
    ],
    key: &["id"],
    regenerate: Some("r"),
    gate: "g",
};

static ARM_T: TableSpec = TableSpec {
    name: "arm",
    owner: OwnerClass::Generated,
    columns: &[Column {
        name: "arm",
        kind: ColumnKind::Ident,
        nullable: false,
        references: None,
    }],
    key: &["arm"],
    regenerate: Some("r"),
    gate: "g",
};

const HEADER: &str = "id\ttag\tarm\tlist\n";

fn with_rows(rows: &str) -> String {
    format!("{HEADER}{rows}")
}

#[test]
fn a_well_formed_table_parses_into_its_rows() {
    let text = with_rows("1\ta\tX_1\t1,2,30\n9\tb\tnone\tnone\n10\ta\tY\t7\n");
    let table = parse(&T, &text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(table.spec.name, "t");
    assert_eq!(
        table.rows,
        vec![
            vec!["1", "a", "X_1", "1,2,30"],
            vec!["9", "b", "none", "none"],
            vec!["10", "a", "Y", "7"],
        ]
    );
}

#[test]
fn each_rule_is_refused_by_name() {
    let cases: Vec<(String, ArchiveError)> = vec![
        (
            with_rows("1\ta\tnone\tnone"),
            ArchiveError::MissingFinalNewline { table: "t" },
        ),
        (
            "id\ttag\tarm\tlist\r\n1\ta\tnone\tnone\n".to_string(),
            ArchiveError::CarriageReturn {
                table: "t",
                line: 1,
            },
        ),
        (
            with_rows("1\ta\tn\u{e9}\tnone\n"),
            ArchiveError::NonAscii {
                table: "t",
                line: 2,
            },
        ),
        (
            "id\ttag\tarm\n1\ta\tnone\n".to_string(),
            ArchiveError::Header {
                table: "t",
                expected: "id\ttag\tarm\tlist".to_string(),
                found: "id\ttag\tarm".to_string(),
            },
        ),
        (
            with_rows("1\ta\tnone\tnone\textra\n"),
            ArchiveError::CellCount {
                table: "t",
                line: 2,
                expected: 4,
                found: 5,
            },
        ),
        (
            with_rows("1\t\tnone\tnone\n"),
            ArchiveError::EmptyCell {
                table: "t",
                line: 2,
                column: "tag",
            },
        ),
        (
            with_rows("1\ta\t\"X\tnone\n"),
            ArchiveError::LeadingQuote {
                table: "t",
                line: 2,
                column: "arm",
            },
        ),
        (
            with_rows("none\ta\tnone\tnone\n"),
            ArchiveError::NoneRefused {
                table: "t",
                line: 2,
                column: "id",
            },
        ),
        (
            with_rows("01\ta\tnone\tnone\n"),
            ArchiveError::BadCell {
                table: "t",
                line: 2,
                column: "id",
                cell: "01".to_string(),
                expected: "a decimal integer".to_string(),
            },
        ),
        (
            with_rows("9223372036854775808\ta\tnone\tnone\n"),
            ArchiveError::BadCell {
                table: "t",
                line: 2,
                column: "id",
                cell: "9223372036854775808".to_string(),
                expected: "a decimal integer".to_string(),
            },
        ),
        (
            with_rows("1\tc\tnone\tnone\n"),
            ArchiveError::BadCell {
                table: "t",
                line: 2,
                column: "tag",
                cell: "c".to_string(),
                expected: "one of a, b".to_string(),
            },
        ),
        (
            with_rows("1\ta\tx-y\tnone\n"),
            ArchiveError::BadCell {
                table: "t",
                line: 2,
                column: "arm",
                cell: "x-y".to_string(),
                expected: "an identifier of letters, digits and _".to_string(),
            },
        ),
        (
            with_rows("1\ta\tnone\t3,2\n"),
            ArchiveError::BadCell {
                table: "t",
                line: 2,
                column: "list",
                cell: "3,2".to_string(),
                expected: "an ascending comma-joined list of decimal integers".to_string(),
            },
        ),
        (
            with_rows("1\ta\tnone\t2,2\n"),
            ArchiveError::BadCell {
                table: "t",
                line: 2,
                column: "list",
                cell: "2,2".to_string(),
                expected: "an ascending comma-joined list of decimal integers".to_string(),
            },
        ),
        (
            with_rows("1\ta\tnone\t1,9223372036854775808\n"),
            ArchiveError::BadCell {
                table: "t",
                line: 2,
                column: "list",
                cell: "1,9223372036854775808".to_string(),
                expected: "an ascending comma-joined list of decimal integers".to_string(),
            },
        ),
        (
            with_rows("2\ta\tnone\tnone\n1\ta\tnone\tnone\n"),
            ArchiveError::Unsorted {
                table: "t",
                line: 3,
            },
        ),
        (
            with_rows("1\ta\tnone\tnone\n1\tb\tnone\tnone\n"),
            ArchiveError::DuplicateKey {
                table: "t",
                line: 3,
            },
        ),
        (
            with_rows("1\ta\tnone\tnone\n\n"),
            ArchiveError::CellCount {
                table: "t",
                line: 3,
                expected: 4,
                found: 1,
            },
        ),
    ];
    for (text, expected) in cases {
        assert_eq!(parse(&T, &text), Err(expected), "text {text:?}");
    }
}

#[test]
fn an_integer_key_sorts_numerically_and_a_text_key_bytewise() {
    assert!(parse(&T, &with_rows("9\ta\tnone\tnone\n10\ta\tnone\tnone\n")).is_ok());
    assert_eq!(
        parse(&ARM_T, "arm\nB\na\nA\n"),
        Err(ArchiveError::Unsorted {
            table: "arm",
            line: 4
        })
    );
    assert!(parse(&ARM_T, "arm\nA\nB\na\n").is_ok());
}

#[test]
fn render_writes_what_parse_reads_and_refuses_what_it_refuses() {
    let rows = vec![
        vec![
            "1".to_string(),
            "a".to_string(),
            "X".to_string(),
            "5".to_string(),
        ],
        vec![
            "2".to_string(),
            "b".to_string(),
            NONE.to_string(),
            NONE.to_string(),
        ],
    ];
    let text = render(&T, &rows).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(text, with_rows("1\ta\tX\t5\n2\tb\tnone\tnone\n"));
    assert_eq!(parse(&T, &text).map(|t| t.rows), Ok(rows));

    let bad = vec![vec![
        "1".to_string(),
        "a".to_string(),
        String::new(),
        "5".to_string(),
    ]];
    assert_eq!(
        render(&T, &bad),
        Err(ArchiveError::EmptyCell {
            table: "t",
            line: 2,
            column: "arm",
        })
    );
}

#[test]
fn references_are_checked_against_the_loaded_target() {
    let arms = parse(&ARM_T, "arm\nX\n").unwrap_or_else(|e| panic!("{e}"));
    let ok = parse(&T, &with_rows("1\ta\tX\tnone\n2\ta\tnone\tnone\n"))
        .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(check_references(&[arms.clone(), ok]), Ok(()));

    let dangling =
        parse(&T, &with_rows("1\ta\tX\tnone\n2\ta\tY\tnone\n")).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        check_references(&[arms, dangling.clone()]),
        Err(ArchiveError::DanglingReference {
            table: "t",
            line: 3,
            column: "arm",
            cell: "Y".to_string(),
            target_table: "arm",
            target_column: "arm",
        })
    );
    assert_eq!(
        check_references(&[dangling]),
        Err(ArchiveError::ReferenceTargetMissing {
            table: "t",
            column: "arm",
            target_table: "arm",
            target_column: "arm",
        })
    );
}
