//! The microtests with no console capture under the reference profile,
//! as `tests/micro/ps3_gaps.tsv` names them. The hardware-capture checks
//! hold the captures they compared to exactly the other cases, so a lost
//! capture or a renamed profile fails instead of comparing nothing.

use std::path::Path;

use super::cases::Case;

/// The ledger's header line.
const HEADER: &str = "name\tclass\treason\tissue";

/// The names in the gap ledger under `micro_root`.
fn gap_names(micro_root: &Path) -> Vec<String> {
    let path = micro_root.join("ps3_gaps.tsv");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let mut lines = text.lines();
    assert_eq!(lines.next(), Some(HEADER), "{} header", path.display());
    lines
        .map(|line| {
            line.split('\t')
                .next()
                .expect("split yields one field")
                .to_string()
        })
        .collect()
}

/// The names of the `cases` the gap ledger does not name, in table
/// order: each must have a capture under the reference profile.
pub fn expected_captures(micro_root: &Path, cases: &[Case]) -> Vec<&'static str> {
    let gaps = gap_names(micro_root);
    cases
        .iter()
        .map(|case| case.name)
        .filter(|name| !gaps.iter().any(|gap| gap == name))
        .collect()
}
