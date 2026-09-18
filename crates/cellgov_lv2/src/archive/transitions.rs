//! Cross-version LV2 census transitions.

use std::collections::BTreeMap;

use super::spec::TRANSITIONS;
use super::table::{self, ArchiveError, NONE};
use super::{CensusClass, CensusRow, FirmwareRow, GateRow, GateState};

/// Classifies whether an adjacent firmware pair has source data on both sides.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonState {
    /// Both firmware versions have extracted census and gate rows.
    Compared,
    /// At least one firmware version lacks extracted source rows.
    NotCompared,
}

impl ComparisonState {
    const fn label(self) -> &'static str {
        match self {
            Self::Compared => "compared",
            Self::NotCompared => "not_compared",
        }
    }
}

/// Names one observed cross-version change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransitionKind {
    /// An absent ordinal gained a target.
    Added,
    /// A target became absent.
    Removed,
    /// An ordinal changed between implementation and stub classes.
    ClassChanged,
    /// An ordinal target moved differently from its surrounding targets.
    Retargeted,
    /// A recognized capability gate appeared.
    GateAdded,
    /// A recognized capability gate disappeared.
    GateRemoved,
}

impl TransitionKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Added => "added",
            Self::Removed => "removed",
            Self::ClassChanged => "class_changed",
            Self::Retargeted => "retargeted",
            Self::GateAdded => "gate_added",
            Self::GateRemoved => "gate_removed",
        }
    }
}

/// Records one adjacent firmware-pair comparison or transition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransitionRow {
    /// Names the earlier firmware version.
    pub from_fw: String,
    /// Names the later firmware version.
    pub to_fw: String,
    /// States whether both versions were extracted.
    pub comparison: ComparisonState,
    /// Names the transition, absent for an uncomparable pair.
    pub kind: Option<TransitionKind>,
    /// Names the changed ordinal, absent for an uncomparable pair.
    pub ordinal: Option<usize>,
}

impl TransitionRow {
    fn cells(&self, record: usize) -> Vec<String> {
        vec![
            record.to_string(),
            self.from_fw.clone(),
            self.to_fw.clone(),
            self.comparison.label().to_string(),
            self.kind
                .map_or_else(|| NONE.to_string(), |kind| kind.label().to_string()),
            self.ordinal
                .map_or_else(|| NONE.to_string(), |ordinal| ordinal.to_string()),
        ]
    }
}

/// Compares adjacent firmware versions in matrix order.
#[must_use]
pub fn transitions(
    firmware: &[FirmwareRow],
    census_by_version: &BTreeMap<String, Vec<CensusRow>>,
    gates_by_version: &BTreeMap<String, Vec<GateRow>>,
) -> Vec<TransitionRow> {
    let mut ordered = firmware.to_vec();
    ordered.sort_by_key(|row| row.order);
    ordered
        .windows(2)
        .flat_map(|pair| compare_pair(&pair[0], &pair[1], census_by_version, gates_by_version))
        .collect()
}

/// Canonicalizes cross-version transition rows by their archive key.
///
/// # Errors
///
/// Returns [`ArchiveError`] when a row violates the frozen schema.
pub fn transitions_tsv(rows: &[TransitionRow]) -> Result<String, ArchiveError> {
    let mut ordered: Vec<&TransitionRow> = rows.iter().collect();
    ordered.sort_by(|left, right| {
        left.from_fw
            .cmp(&right.from_fw)
            .then_with(|| left.to_fw.cmp(&right.to_fw))
            .then_with(|| left.comparison.label().cmp(right.comparison.label()))
            .then_with(|| {
                left.kind
                    .map(TransitionKind::label)
                    .cmp(&right.kind.map(TransitionKind::label))
            })
            .then_with(|| left.ordinal.cmp(&right.ordinal))
    });
    let cells: Vec<Vec<String>> = ordered
        .iter()
        .enumerate()
        .map(|(record, row)| row.cells(record))
        .collect();
    table::render(&TRANSITIONS, &cells)
}

fn compare_pair(
    from: &FirmwareRow,
    to: &FirmwareRow,
    census_by_version: &BTreeMap<String, Vec<CensusRow>>,
    gates_by_version: &BTreeMap<String, Vec<GateRow>>,
) -> Vec<TransitionRow> {
    let Some(before) = census_by_version.get(&from.fw) else {
        return vec![not_compared(from, to)];
    };
    let Some(after) = census_by_version.get(&to.fw) else {
        return vec![not_compared(from, to)];
    };
    let Some(before_gates) = gates_by_version.get(&from.fw) else {
        return vec![not_compared(from, to)];
    };
    let Some(after_gates) = gates_by_version.get(&to.fw) else {
        return vec![not_compared(from, to)];
    };
    if before.len() != after.len()
        || before_gates.len() != before.len()
        || after_gates.len() != after.len()
    {
        return vec![not_compared(from, to)];
    }
    let mut rows = Vec::new();
    for ordinal in 0..before.len() {
        let before_row = &before[ordinal];
        let after_row = &after[ordinal];
        if let Some(kind) = class_transition(before_row.class, after_row.class) {
            rows.push(compared(from, to, kind, ordinal));
        }
        if before_row.class == after_row.class
            && before_row.target != after_row.target
            && !moved_with_neighbours(before, after, ordinal)
        {
            rows.push(compared(from, to, TransitionKind::Retargeted, ordinal));
        }
        // `NotAnalysed` makes no claim, so only two classified states can
        // establish that a gate appeared or disappeared.
        match (before_gates[ordinal].state, after_gates[ordinal].state) {
            (GateState::Ungated, GateState::Gated) => {
                rows.push(compared(from, to, TransitionKind::GateAdded, ordinal));
            }
            (GateState::Gated, GateState::Ungated) => {
                rows.push(compared(from, to, TransitionKind::GateRemoved, ordinal));
            }
            _ => {}
        }
    }
    rows
}

fn class_transition(before: CensusClass, after: CensusClass) -> Option<TransitionKind> {
    match (before, after) {
        (CensusClass::Absent, CensusClass::Absent)
        | (CensusClass::Implemented, CensusClass::Implemented)
        | (CensusClass::Stub, CensusClass::Stub) => None,
        (CensusClass::Absent, _) => Some(TransitionKind::Added),
        (_, CensusClass::Absent) => Some(TransitionKind::Removed),
        _ => Some(TransitionKind::ClassChanged),
    }
}

fn moved_with_neighbours(before: &[CensusRow], after: &[CensusRow], ordinal: usize) -> bool {
    let Some(shift) = target_shift(&before[ordinal], &after[ordinal]) else {
        return false;
    };
    let mut neighbours = 0;
    for neighbour in [ordinal.checked_sub(1), ordinal.checked_add(1)] {
        let Some(index) = neighbour.filter(|index| *index < before.len()) else {
            continue;
        };
        if target_shift(&before[index], &after[index]) == Some(shift) {
            neighbours += 1;
        }
    }
    neighbours > 0
}

fn target_shift(before: &CensusRow, after: &CensusRow) -> Option<i128> {
    Some(i128::from(after.target?) - i128::from(before.target?))
}

fn not_compared(from: &FirmwareRow, to: &FirmwareRow) -> TransitionRow {
    TransitionRow {
        from_fw: from.fw.clone(),
        to_fw: to.fw.clone(),
        comparison: ComparisonState::NotCompared,
        kind: None,
        ordinal: None,
    }
}

fn compared(
    from: &FirmwareRow,
    to: &FirmwareRow,
    kind: TransitionKind,
    ordinal: usize,
) -> TransitionRow {
    TransitionRow {
        from_fw: from.fw.clone(),
        to_fw: to.fw.clone(),
        comparison: ComparisonState::Compared,
        kind: Some(kind),
        ordinal: Some(ordinal),
    }
}

#[cfg(test)]
#[path = "tests/transitions_tests.rs"]
mod tests;
