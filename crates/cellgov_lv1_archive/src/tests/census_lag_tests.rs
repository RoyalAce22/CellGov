use super::*;
use cellgov_lv2_archive::parse;

fn set(numbers: &[u64]) -> BTreeSet<u64> {
    numbers.iter().copied().collect()
}

/// A case: what it shows, the census's answered numbers, the registered
/// numbers and the contradicted numbers.
type Case = (&'static str, &'static [u64], &'static [u64], &'static [u64]);

#[test]
fn the_census_may_lag_the_registrations_and_never_contradict_them() {
    let cases: &[Case] = &[
        (
            "the census and the registrations agree",
            &[0, 74],
            &[0, 74],
            &[],
        ),
        (
            "a number registered since the last regeneration",
            &[0],
            &[0, 74],
            &[],
        ),
        (
            "an empty census lags every registration",
            &[],
            &[0, 74],
            &[],
        ),
        (
            "an answered number that no registration declares",
            &[0, 74],
            &[0],
            &[74],
        ),
        (
            "two answered numbers that no registration declares",
            &[0, 3, 74],
            &[74],
            &[0, 3],
        ),
    ];
    assert_eq!(cases.len(), 5);
    for (what, census, registered, contradicted) in cases {
        assert_eq!(
            census_contradictions(&set(census), &set(registered)),
            *contradicted,
            "{what}"
        );
    }
}

#[test]
fn census_answered_reads_the_numbers_whose_state_is_answered() {
    let rows = [
        (0, HypercallState::Answered),
        (5, HypercallState::NullBackend),
        (74, HypercallState::Answered),
    ];
    let rows: Vec<HypercallRow> = rows
        .iter()
        .map(|&(number, state)| HypercallRow {
            number,
            name: None,
            inputs: None,
            outputs: None,
            family: "unnamed",
            leaf: "boot",
            fw: Vec::new(),
            sites: 0,
            lv1_493: Lv1Entry::Handler,
            state,
        })
        .collect();
    let text = hypercall_tsv(&rows).expect("the rows fit the schema");
    let table = parse(&HYPERCALL, &text).expect("the rendered table parses");
    assert_eq!(census_answered(&table), Ok(set(&[0, 74])));
}

#[test]
fn census_answered_refuses_a_table_the_hypercall_spec_did_not_check() {
    let row = HypercallRow {
        number: 74,
        name: None,
        inputs: None,
        outputs: None,
        family: "unnamed",
        leaf: "boot",
        fw: Vec::new(),
        sites: 0,
        lv1_493: Lv1Entry::Handler,
        state: HypercallState::Answered,
    };
    let text = hypercall_tsv(&[row]).expect("the row fits the schema");
    let good = parse(&HYPERCALL, &text).expect("the rendered table parses");
    let mut bad_number = good.rows.clone();
    bad_number[0][0] = "seventy".to_string();
    let cases: &[(&str, Table, &str)] = &[
        (
            "a table of another spec",
            Table {
                spec: &HYPERCALL_SITE,
                rows: Vec::new(),
            },
            "header",
        ),
        (
            "a row with one cell",
            Table {
                spec: &HYPERCALL,
                rows: vec![vec!["74".to_string()]],
            },
            "cell count",
        ),
        (
            "an answered row whose number is no integer",
            Table {
                spec: &HYPERCALL,
                rows: bad_number,
            },
            "bad cell",
        ),
    ];
    assert_eq!(cases.len(), 3);
    for (what, table, refusal) in cases {
        let found = match census_answered(table) {
            Err(ArchiveError::Header { .. }) => "header",
            Err(ArchiveError::CellCount { line: 2, .. }) => "cell count",
            Err(ArchiveError::BadCell {
                line: 2,
                column: "number",
                ..
            }) => "bad cell",
            other => panic!("{what}: {other:?}"),
        };
        assert_eq!(found, *refusal, "{what}");
    }
}
