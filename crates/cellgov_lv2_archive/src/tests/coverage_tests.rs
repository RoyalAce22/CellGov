use super::*;
use crate::DispatchShape;

fn census(fw: &str, classes: &[CensusClass]) -> Vec<CensusRow> {
    classes
        .iter()
        .enumerate()
        .map(|(ordinal, class)| CensusRow {
            fw: fw.to_string(),
            ordinal,
            class: *class,
            target: None,
            dispatch: DispatchShape::Flat,
        })
        .collect()
}

#[test]
fn implemented_and_total_denominators_stay_distinct() {
    let routes = vec![
        RouteRow {
            ordinal: 0,
            route: Route::Typed,
            arm: Some("A"),
        },
        RouteRow {
            ordinal: 1,
            route: Route::NullBackend,
            arm: None,
        },
        RouteRow {
            ordinal: 2,
            route: Route::Routed,
            arm: Some("B"),
        },
    ];
    let census_by_version = BTreeMap::from([
        (
            "3.55".to_string(),
            census(
                "3.55",
                &[
                    CensusClass::Implemented,
                    CensusClass::Stub,
                    CensusClass::Absent,
                ],
            ),
        ),
        (
            "3.56".to_string(),
            census(
                "3.56",
                &[
                    CensusClass::Implemented,
                    CensusClass::Stub,
                    CensusClass::Stub,
                ],
            ),
        ),
    ]);
    assert_eq!(
        coverage_rows(&routes, &census_by_version),
        [
            CoverageRow {
                scope: CoverageScope::Implemented,
                extracted: Some(1),
                handled: Some(1),
                versions: 2
            },
            CoverageRow {
                scope: CoverageScope::Total,
                extracted: Some(3),
                handled: Some(2),
                versions: 2
            },
        ]
    );
}

#[test]
fn an_empty_census_reports_unavailable_denominators() {
    assert_eq!(
        coverage_rows(&[], &BTreeMap::new()),
        [
            CoverageRow {
                scope: CoverageScope::Implemented,
                extracted: None,
                handled: None,
                versions: 0
            },
            CoverageRow {
                scope: CoverageScope::Total,
                extracted: None,
                handled: None,
                versions: 0
            },
        ]
    );
}

#[test]
fn coverage_renderer_keeps_unavailable_denominators_explicit() {
    assert_eq!(
        coverage_tsv(&[
            CoverageRow {
                scope: CoverageScope::Implemented,
                extracted: None,
                handled: None,
                versions: 0
            },
            CoverageRow {
                scope: CoverageScope::Total,
                extracted: Some(3),
                handled: Some(2),
                versions: 2
            },
        ])
        .expect("render coverage"),
        concat!(
            "scope\textracted\thandled\tversions\n",
            "implemented\tnone\tnone\t0\n",
            "total\t3\t2\t2\n"
        )
    );
}
