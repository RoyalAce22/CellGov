//! Byte-level memory-region diff.

use crate::observation::NamedMemoryRegion;

use super::types::MemoryDivergence;

/// Regions match by name; a region in one side but not the other diverges at offset 0.
///
/// A region present on both sides diverges on its first differing byte
/// or, failing that, on a length disagreement: the walk reads a short
/// side as zeros past its end, so an all-zero surplus produces no
/// differing byte even though the two sides describe different amounts
/// of guest state.
pub(super) fn find_memory_divergence(
    expected: &[NamedMemoryRegion],
    actual: &[NamedMemoryRegion],
) -> Option<MemoryDivergence> {
    for exp in expected {
        let Some(act) = actual.iter().find(|r| r.name == exp.name) else {
            return Some(MemoryDivergence {
                region: exp.name.clone(),
                offset: 0,
                expected: exp.data.first().copied().unwrap_or(0),
                actual: 0,
                lengths: None,
            });
        };
        if let Some(divergence) = region_divergence(exp, act) {
            return Some(divergence);
        }
    }
    for act in actual {
        if !expected.iter().any(|r| r.name == act.name) {
            return Some(MemoryDivergence {
                region: act.name.clone(),
                offset: 0,
                expected: 0,
                actual: act.data.first().copied().unwrap_or(0),
                lengths: None,
            });
        }
    }
    None
}

/// The first divergence between two regions of one name: a differing
/// byte under zero padding, else the end of the shorter side.
fn region_divergence(exp: &NamedMemoryRegion, act: &NamedMemoryRegion) -> Option<MemoryDivergence> {
    let lengths = (exp.data.len() != act.data.len()).then_some((exp.data.len(), act.data.len()));
    let len = exp.data.len().max(act.data.len());
    for i in 0..len {
        let e = exp.data.get(i).copied().unwrap_or(0);
        let a = act.data.get(i).copied().unwrap_or(0);
        if e != a {
            return Some(MemoryDivergence {
                region: exp.name.clone(),
                offset: i,
                expected: e,
                actual: a,
                lengths,
            });
        }
    }
    // Every byte agreed under zero padding, so the only thing left to
    // report is where the shorter side ran out.
    let (exp_len, act_len) = lengths?;
    let offset = exp_len.min(act_len);
    Some(MemoryDivergence {
        region: exp.name.clone(),
        offset,
        expected: exp.data.get(offset).copied().unwrap_or(0),
        actual: act.data.get(offset).copied().unwrap_or(0),
        lengths,
    })
}

#[cfg(test)]
#[path = "tests/memory_tests.rs"]
mod tests;
