use std::collections::BTreeSet;

use cellgov_ps3_abi::lv2::syscall;
use strum::VariantArray;

use super::*;
use crate::table::{check_references, parse};
use cellgov_lv2::request::Lv2RequestKind;

#[test]
fn route_rows_cover_every_slot_once_in_order() {
    let routes = route_rows();
    let ordinals: Vec<u64> = routes.iter().map(|r| r.ordinal).collect();
    let expected: Vec<u64> = (0..SYSCALL_TABLE_SLOTS).collect();
    assert_eq!(ordinals, expected);
}

#[test]
fn the_timer_slots_read_runtime_fast_path_and_nothing_else_does() {
    let routes = route_rows();
    let fast: Vec<u64> = routes
        .iter()
        .filter(|r| r.route == Route::RuntimeFastPath)
        .map(|r| r.ordinal)
        .collect();
    assert_eq!(fast, RUNTIME_FAST_PATH);
    assert_eq!(fast, [syscall::TIMER_USLEEP, syscall::TIMER_SLEEP]);
    for row in routes.iter().filter(|r| r.route == Route::RuntimeFastPath) {
        assert_eq!(row.arm, None, "slot {} names an arm", row.ordinal);
    }
    let counts = HandlingCounts::of(&routes);
    assert_eq!(counts.runtime_fast_path, RUNTIME_FAST_PATH.len());
    assert_eq!(
        counts.typed + counts.routed + counts.null_backend + counts.runtime_fast_path,
        routes.len()
    );
    for route in Route::ALL {
        assert_eq!(
            counts.of_route(*route),
            routes.iter().filter(|r| r.route == *route).count()
        );
    }
}

#[test]
fn a_typed_slot_names_its_variant_and_a_routed_slot_its_arm() {
    let routes = route_rows();
    let by_ordinal = |n: u64| {
        routes
            .iter()
            .find(|r| r.ordinal == n)
            .unwrap_or_else(|| panic!("no row for {n}"))
    };
    let exit = by_ordinal(syscall::PROCESS_EXIT);
    assert_eq!((exit.route, exit.arm), (Route::Typed, Some("ProcessExit")));
    let tty = by_ordinal(syscall::TTY_READ);
    assert_eq!((tty.route, tty.arm), (Route::Routed, Some("TtyRead")));
    let null = by_ordinal(999);
    assert_eq!((null.route, null.arm), (Route::NullBackend, None));
    for row in &routes {
        assert_eq!(
            row.arm.is_some(),
            matches!(row.route, Route::Typed | Route::Routed),
            "slot {}: arm and route disagree",
            row.ordinal
        );
    }
}

#[test]
fn process_spawn_serves_two_slots_and_process_is_stack_none() {
    let routes = route_rows();
    let arms = arm_rows(&routes);
    let find = |name: &str| {
        arms.iter()
            .find(|a| a.arm == name)
            .unwrap_or_else(|| panic!("no arm row {name}"))
    };
    assert_eq!(
        find("ProcessSpawn").ordinals,
        [syscall::PROCESS_SPAWN, syscall::PROCESS_SPAWNS_A_SELF2]
    );
    assert!(find("ProcessIsStack").ordinals.is_empty());
}

#[test]
fn arm_rows_are_sorted_unique_and_cover_every_tagged_variant_and_routed_arm() {
    let routes = route_rows();
    let arms = arm_rows(&routes);
    let names: Vec<&str> = arms.iter().map(|a| a.arm).collect();
    let mut sorted = names.clone();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(names, sorted);
    let tagged = Lv2RequestKind::VARIANTS
        .iter()
        .filter(|k| k.fidelity().is_some())
        .count();
    assert_eq!(arms.len(), tagged + ROUTED_UNSUPPORTED_ARMS.len());
    for kind in Lv2RequestKind::VARIANTS {
        let name: &str = (*kind).into();
        assert_eq!(names.contains(&name), kind.fidelity().is_some(), "{name}");
    }
    for (n, arm, fidelity) in ROUTED_UNSUPPORTED_ARMS {
        let row = arms
            .iter()
            .find(|a| a.arm == *arm)
            .unwrap_or_else(|| panic!("no arm row {arm}"));
        assert_eq!(row.fidelity, *fidelity);
        // A routed number that `classify` also types, or that the fast
        // path shadows, renders the arm with no slot and fails nowhere
        // else.
        assert_eq!(row.ordinals, [*n], "routed arm {arm} lost its slot");
    }
}

#[test]
fn the_slots_an_arm_serves_are_exactly_the_slots_that_reach_it() {
    let routes = route_rows();
    let arms = arm_rows(&routes);
    let mut served = BTreeSet::new();
    for arm in &arms {
        for ordinal in &arm.ordinals {
            assert!(served.insert(*ordinal), "slot {ordinal} served twice");
            let row = routes
                .iter()
                .find(|r| r.ordinal == *ordinal)
                .unwrap_or_else(|| panic!("no row for {ordinal}"));
            assert_eq!(row.arm, Some(arm.arm));
        }
    }
    let reaching: BTreeSet<u64> = routes
        .iter()
        .filter(|r| r.arm.is_some())
        .map(|r| r.ordinal)
        .collect();
    assert_eq!(served, reaching);
}

#[test]
fn the_rendered_tables_load_and_reference_each_other() {
    let routes = route_rows();
    let arms = arm_rows(&routes);
    let route_text = route_tsv(&routes).unwrap_or_else(|e| panic!("{e}"));
    let arm_text = arm_tsv(&arms).unwrap_or_else(|e| panic!("{e}"));
    let route_table = parse(&ROUTE, &route_text).unwrap_or_else(|e| panic!("{e}"));
    let arm_table = parse(&ARM, &arm_text).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(route_table.rows.len(), routes.len());
    assert_eq!(arm_table.rows.len(), arms.len());
    assert_eq!(check_references(&[arm_table, route_table]), Ok(()));
    assert!(route_text.contains("\n141\truntime_fast_path\tnone\n"));
    assert!(arm_text.contains("\nProcessSpawn\tpartial-state\t21,27\n"));
    assert!(arm_text.contains("\nProcessIsStack\tmodeled\tnone\n"));
}
