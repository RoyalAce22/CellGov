//! The caller tables: typed rows, their renderers' order, and the merge.

use super::*;
use crate::parse;

fn pup(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn caller(pup_sha256: &str, module: &str, ordinal: usize, sites: &[u64]) -> CallerRow {
    CallerRow {
        pup_sha256: pup_sha256.to_string(),
        module: module.to_string(),
        ordinal,
        sites: sites.to_vec(),
    }
}

fn unresolved(pup_sha256: &str, module: &str, sites: &[u64]) -> CallerUnresolvedRow {
    CallerUnresolvedRow {
        pup_sha256: pup_sha256.to_string(),
        module: module.to_string(),
        sites: sites.to_vec(),
    }
}

fn reach(pup_sha256: &str, module: &str, export_nid: u64, ordinal: usize) -> ReachRow {
    ReachRow {
        pup_sha256: pup_sha256.to_string(),
        module: module.to_string(),
        export_nid,
        ordinal,
    }
}

#[test]
fn each_table_round_trips_its_typed_rows() {
    let callers = vec![
        caller(&pup(1), "sys/a.sprx", 7, &[16, 32]),
        caller(&pup(1), "sys/a.sprx", 22, &[48]),
    ];
    let text = caller_tsv(&callers).unwrap();
    assert_eq!(caller_rows(&parse(&CALLER, &text).unwrap()), callers);

    let unresolved_rows = vec![
        unresolved(&pup(1), "sys/a.sprx", &[]),
        unresolved(&pup(1), "sys/b.sprx", &[4, 8]),
    ];
    let text = caller_unresolved_tsv(&unresolved_rows).unwrap();
    assert!(text.contains("\tnone\n"), "{text}");
    assert_eq!(
        caller_unresolved_rows(&parse(&CALLER_UNRESOLVED, &text).unwrap()),
        unresolved_rows
    );

    let reach_rows_in = vec![
        reach(&pup(1), "sys/a.sprx", 7, 22),
        reach(&pup(1), "sys/a.sprx", 30, 7),
    ];
    let text = reach_tsv(&reach_rows_in).unwrap();
    assert_eq!(reach_rows(&parse(&REACH, &text).unwrap()), reach_rows_in);
}

/// The column is a plain integer, so a held row may carry a NID wider
/// than 32 bits; reading and rendering the row keep it unchanged.
#[test]
fn a_held_reach_row_wider_than_a_nid_reads_back_unchanged() {
    let held = reach_tsv(&[reach(&pup(1), "sys/a.sprx", 1 << 32, 7)]).unwrap();
    let rows = reach_rows(&parse(&REACH, &held).unwrap());
    assert_eq!(rows[0].export_nid, 4_294_967_296);
    assert_eq!(reach_tsv(&rows).unwrap(), held);
}

#[test]
fn the_renderers_order_integer_keys_numerically() {
    // The rows arrive out of order, and "22" sorts before "7" as text.
    let callers = [
        caller(&pup(2), "sys/a.sprx", 1, &[1]),
        caller(&pup(1), "sys/b.sprx", 1, &[1]),
        caller(&pup(1), "sys/a.sprx", 22, &[1]),
        caller(&pup(1), "sys/a.sprx", 7, &[1]),
    ];
    let rows = caller_rows(&parse(&CALLER, &caller_tsv(&callers).unwrap()).unwrap());
    let keys: Vec<(String, &str, usize)> = rows
        .iter()
        .map(|row| (row.pup_sha256.clone(), row.module.as_str(), row.ordinal))
        .collect();
    assert_eq!(
        keys,
        [
            (pup(1), "sys/a.sprx", 7),
            (pup(1), "sys/a.sprx", 22),
            (pup(1), "sys/b.sprx", 1),
            (pup(2), "sys/a.sprx", 1),
        ]
    );

    let reaches = [
        reach(&pup(1), "sys/a.sprx", 22, 3),
        reach(&pup(1), "sys/a.sprx", 7, 30),
        reach(&pup(1), "sys/a.sprx", 7, 4),
    ];
    let rows = reach_rows(&parse(&REACH, &reach_tsv(&reaches).unwrap()).unwrap());
    let keys: Vec<(u64, usize)> = rows
        .iter()
        .map(|row| (row.export_nid, row.ordinal))
        .collect();
    assert_eq!(keys, [(7, 4), (7, 30), (22, 3)]);

    let unresolveds = [
        unresolved(&pup(2), "sys/a.sprx", &[]),
        unresolved(&pup(1), "sys/b.sprx", &[]),
        unresolved(&pup(1), "sys/a.sprx", &[]),
    ];
    let rows = caller_unresolved_rows(
        &parse(
            &CALLER_UNRESOLVED,
            &caller_unresolved_tsv(&unresolveds).unwrap(),
        )
        .unwrap(),
    );
    let keys: Vec<(String, &str)> = rows
        .iter()
        .map(|row| (row.pup_sha256.clone(), row.module.as_str()))
        .collect();
    assert_eq!(
        keys,
        [
            (pup(1), "sys/a.sprx"),
            (pup(1), "sys/b.sprx"),
            (pup(2), "sys/a.sprx"),
        ]
    );
}

#[test]
fn a_rescan_keeps_other_valid_pups_rows_and_drops_its_own_and_unknown_ones() {
    let (a, b, gone) = (pup(1), pup(2), pup(3));
    let existing = CallerCensus {
        caller: vec![
            caller(&a, "sys/a.sprx", 22, &[100]),
            caller(&b, "sys/a.sprx", 22, &[100]),
            caller(&gone, "sys/a.sprx", 22, &[100]),
        ],
        unresolved: vec![
            unresolved(&a, "sys/a.sprx", &[]),
            unresolved(&b, "sys/a.sprx", &[]),
            unresolved(&gone, "sys/a.sprx", &[]),
        ],
        reach: vec![
            reach(&a, "sys/a.sprx", 33, 22),
            reach(&b, "sys/a.sprx", 33, 22),
            reach(&gone, "sys/a.sprx", 33, 22),
        ],
    };
    // The rescan of `a` resolves no site, so only its unresolved row
    // marks `a` as rescanned.
    let mut refreshed = CallerCensus {
        caller: Vec::new(),
        unresolved: vec![unresolved(&a, "sys/a.sprx", &[200])],
        reach: Vec::new(),
    };
    let valid: BTreeSet<&str> = [a.as_str(), b.as_str()].into_iter().collect();
    refreshed.merge_existing(existing, &valid);
    assert_eq!(refreshed.caller, [caller(&b, "sys/a.sprx", 22, &[100])]);
    assert_eq!(
        refreshed.unresolved,
        [
            unresolved(&a, "sys/a.sprx", &[200]),
            unresolved(&b, "sys/a.sprx", &[])
        ]
    );
    assert_eq!(refreshed.reach, [reach(&b, "sys/a.sprx", 33, 22)]);
}

#[test]
fn a_caller_or_reach_row_alone_also_marks_its_pup_rescanned() {
    let (a, b) = (pup(1), pup(2));
    let existing = CallerCensus {
        caller: vec![
            caller(&a, "sys/a.sprx", 22, &[100]),
            caller(&b, "sys/a.sprx", 22, &[100]),
        ],
        unresolved: Vec::new(),
        reach: Vec::new(),
    };
    let valid: BTreeSet<&str> = [a.as_str(), b.as_str()].into_iter().collect();
    let mut by_caller = CallerCensus {
        caller: vec![caller(&a, "sys/a.sprx", 7, &[16])],
        ..CallerCensus::default()
    };
    by_caller.merge_existing(existing.clone(), &valid);
    assert_eq!(
        by_caller.caller,
        [
            caller(&a, "sys/a.sprx", 7, &[16]),
            caller(&b, "sys/a.sprx", 22, &[100])
        ]
    );
    let mut by_reach = CallerCensus {
        reach: vec![reach(&a, "sys/a.sprx", 33, 22)],
        ..CallerCensus::default()
    };
    by_reach.merge_existing(existing, &valid);
    assert_eq!(by_reach.caller, [caller(&b, "sys/a.sprx", 22, &[100])]);
}
