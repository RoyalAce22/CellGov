use super::*;
use crate::{DispatchShape, FirmwareRole};

fn firmware(fw: &str, order: u64) -> FirmwareRow {
    FirmwareRow {
        fw: fw.to_string(),
        order,
        release_date: None,
        priority: 1,
        role: FirmwareRole::None,
    }
}

fn census(fw: &str, classes: &[(CensusClass, Option<u64>)]) -> Vec<CensusRow> {
    classes
        .iter()
        .enumerate()
        .map(|(ordinal, (class, target))| CensusRow {
            fw: fw.to_string(),
            ordinal,
            class: *class,
            target: *target,
            dispatch: DispatchShape::Flat,
        })
        .collect()
}

fn gates(fw: &str, states: &[GateState]) -> Vec<GateRow> {
    states
        .iter()
        .enumerate()
        .map(|(ordinal, state)| GateRow {
            pup_sha256: format!("{fw:0>64}"),
            ordinal,
            state: *state,
            reads: None,
            fail_errno: None,
        })
        .collect()
}

#[test]
fn matrix_order_drives_adjacency_and_keeps_missing_pairs_explicit() {
    let firmware = vec![firmware("3.56", 356), firmware("3.55", 355)];
    let census_by_version = BTreeMap::from([(
        "3.55".to_string(),
        census("3.55", &[(CensusClass::Implemented, Some(0x1000))]),
    )]);
    assert_eq!(
        transitions(&firmware, &census_by_version, &BTreeMap::new()),
        [TransitionRow {
            from_fw: "3.55".to_string(),
            to_fw: "3.56".to_string(),
            comparison: ComparisonState::NotCompared,
            kind: None,
            ordinal: None
        }]
    );
}

#[test]
fn transitions_distinguish_relocation_retarget_class_and_gate_changes() {
    let firmware = vec![firmware("3.55", 355), firmware("3.56", 356)];
    let census_by_version = BTreeMap::from([
        (
            "3.55".to_string(),
            census(
                "3.55",
                &[
                    (CensusClass::Implemented, Some(0x100)),
                    (CensusClass::Stub, Some(0x200)),
                    (CensusClass::Implemented, Some(0x300)),
                    (CensusClass::Absent, None),
                ],
            ),
        ),
        (
            "3.56".to_string(),
            census(
                "3.56",
                &[
                    (CensusClass::Implemented, Some(0x110)),
                    (CensusClass::Implemented, Some(0x210)),
                    (CensusClass::Implemented, Some(0x360)),
                    (CensusClass::Implemented, Some(0x410)),
                ],
            ),
        ),
    ]);
    let gates_by_version = BTreeMap::from([
        (
            "3.55".to_string(),
            gates(
                "3.55",
                &[
                    GateState::Ungated,
                    GateState::Gated,
                    GateState::NotAnalysed,
                    GateState::NotAnalysed,
                ],
            ),
        ),
        (
            "3.56".to_string(),
            gates(
                "3.56",
                &[
                    GateState::Gated,
                    GateState::Ungated,
                    GateState::NotAnalysed,
                    GateState::NotAnalysed,
                ],
            ),
        ),
    ]);
    let rows = transitions(&firmware, &census_by_version, &gates_by_version);
    assert!(rows
        .iter()
        .any(|row| row.kind == Some(TransitionKind::GateAdded) && row.ordinal == Some(0)));
    assert!(rows
        .iter()
        .any(|row| row.kind == Some(TransitionKind::GateRemoved) && row.ordinal == Some(1)));
    assert!(rows
        .iter()
        .any(|row| row.kind == Some(TransitionKind::ClassChanged) && row.ordinal == Some(1)));
    assert!(rows
        .iter()
        .any(|row| row.kind == Some(TransitionKind::Retargeted) && row.ordinal == Some(2)));
    assert!(rows
        .iter()
        .any(|row| row.kind == Some(TransitionKind::Added) && row.ordinal == Some(3)));
    assert!(!rows
        .iter()
        .any(|row| row.kind == Some(TransitionKind::Retargeted) && row.ordinal == Some(0)));
}

#[test]
fn not_analysed_gate_rows_do_not_fabricate_gate_transitions() {
    let firmware = vec![firmware("3.55", 355), firmware("3.56", 356)];
    let census_by_version = BTreeMap::from([
        (
            "3.55".to_string(),
            census("3.55", &[(CensusClass::Implemented, Some(0x100))]),
        ),
        (
            "3.56".to_string(),
            census("3.56", &[(CensusClass::Implemented, Some(0x100))]),
        ),
    ]);
    let gates_by_version = BTreeMap::from([
        ("3.55".to_string(), gates("3.55", &[GateState::NotAnalysed])),
        ("3.56".to_string(), gates("3.56", &[GateState::Gated])),
    ]);

    assert!(transitions(&firmware, &census_by_version, &gates_by_version).is_empty());
}

#[test]
fn transition_renderer_keeps_not_compared_pairs_explicit() {
    let row = TransitionRow {
        from_fw: "3.55".to_string(),
        to_fw: "3.56".to_string(),
        comparison: ComparisonState::NotCompared,
        kind: None,
        ordinal: None,
    };
    assert_eq!(
        transitions_tsv(&[row]).expect("render transitions"),
        concat!(
            "record\tfrom_fw\tto_fw\tcomparison\tkind\tordinal\n",
            "0\t3.55\t3.56\tnot_compared\tnone\tnone\n"
        )
    );
}
