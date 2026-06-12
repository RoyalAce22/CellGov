//! Byte-level memory-region diff.

use crate::observation::NamedMemoryRegion;

use super::types::MemoryDivergence;

/// Regions match by name; a region in one side but not the other diverges at offset 0.
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
                });
            }
            Some(act) => {
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
                        });
                    }
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
            });
        }
    }
    None
}

#[cfg(test)]
#[path = "tests/memory_tests.rs"]
mod tests;
