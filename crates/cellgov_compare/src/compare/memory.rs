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
        let act = actual.iter().find(|r| r.name == exp.name);
        match act {
            None => {
                return Some(MemoryDivergence {
                    region: exp.name.clone(),
                    offset: 0,
                    expected: exp.data.first().copied().unwrap_or(0),
                    actual: 0,
                    lengths: None,
                });
            }
            Some(act) => {
                let lengths =
                    (exp.data.len() != act.data.len()).then_some((exp.data.len(), act.data.len()));
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
                if let Some((exp_len, act_len)) = lengths {
                    // Every byte agreed under zero padding, so the only
                    // thing left to report is where the shorter side ran
                    // out.
                    let offset = exp_len.min(act_len);
                    return Some(MemoryDivergence {
                        region: exp.name.clone(),
                        offset,
                        expected: exp.data.get(offset).copied().unwrap_or(0),
                        actual: act.data.get(offset).copied().unwrap_or(0),
                        lengths,
                    });
                }
            }
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

#[cfg(test)]
#[path = "tests/memory_tests.rs"]
mod tests;
