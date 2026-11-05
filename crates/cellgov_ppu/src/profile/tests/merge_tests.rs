//! Two reports merge row by row on each label.

use super::{Coverage, InsnClass, OperandProfile, PairClass, ProfileReport};

fn bytes(words: &[u32]) -> Vec<u8> {
    words.iter().flat_map(|w| w.to_be_bytes()).collect()
}

/// `addi r3, r3, 1`.
const ADDI: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1;
/// `addi r4, r4, 0`: a class of its own, its immediate zero.
const ADDI_ZERO: u32 = (14 << 26) | (4 << 21) | (4 << 16);

/// The report of `words` with the word at each pc of `pcs` retired once.
fn report(words: &[u32], pcs: &[u64]) -> ProfileReport {
    let text = bytes(words);
    let mut profile = OperandProfile::default();
    for &pc in pcs {
        profile.record(pc);
    }
    ProfileReport::new(&profile, text.as_slice(), &text, None)
}

#[test]
fn merged_reports_sum_each_row_and_rerank() {
    // One space retires its addi once. Another space retires its zero
    // addi twice at the same pc, and holds an addi it never runs.
    let mut merged = report(&[ADDI], &[0]);
    merged.merge(report(&[ADDI_ZERO, ADDI], &[0, 0]));

    let nonzero = InsnClass::of(ADDI).unwrap();
    let zero = InsnClass::of(ADDI_ZERO).unwrap();
    assert_eq!(merged.retired, 3);
    assert_eq!(merged.insns, vec![(zero, 2, 1), (nonzero, 1, 2)]);
    assert_eq!(
        merged.pairs.iter().map(|(_, n)| n).sum::<u64>(),
        0,
        "pc 0 to pc 0 is a jump, so neither report counts a pair"
    );
}

#[test]
fn merged_pairs_sum_on_each_pair_class() {
    // One space runs its two addis in sequence once, and another space
    // runs them twice.
    let mut merged = report(&[ADDI, ADDI], &[0, 4]);
    merged.merge(report(&[ADDI, ADDI], &[0, 4, 0, 4]));

    let pair = PairClass::of(ADDI, ADDI).unwrap();
    assert_eq!(merged.pairs, vec![(pair, 3)]);
}

#[test]
fn merged_shipped_coverage_sums_where_present() {
    let cover = |retired, covered, removed| Coverage {
        retired,
        covered,
        dispatches_removed: removed,
    };
    let a = ProfileReport {
        shipped: Some(cover(10, 4, 1)),
        shipped_forms: vec![("Li".to_string(), 3), ("LwzCmpwi".to_string(), 1)],
        ..ProfileReport::default()
    };
    let b = ProfileReport {
        shipped: Some(cover(5, 2, 2)),
        shipped_forms: vec![("LwzCmpwi".to_string(), 4)],
        ..ProfileReport::default()
    };

    // A report with no shadow merges on both sides of the two that have one.
    let mut total = ProfileReport::default();
    total.merge(a);
    total.merge(b);
    total.merge(ProfileReport::default());

    assert_eq!(total.shipped, Some(cover(15, 6, 3)));
    assert_eq!(
        total.shipped_forms,
        vec![("LwzCmpwi".to_string(), 5), ("Li".to_string(), 3)]
    );
}
