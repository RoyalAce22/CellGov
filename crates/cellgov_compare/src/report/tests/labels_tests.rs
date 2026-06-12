//! Totality and distinctness of mode and classification label strings.

use super::*;
use strum::VariantArray;

/// Trip-wire: `mode_str` produces a distinct kebab string for
/// every CompareMode. Iterates VARIANTS so a new variant is
/// auto-covered.
#[test]
fn mode_str_is_total_and_distinct() {
    let mut seen: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for m in CompareMode::VARIANTS {
        let s = mode_str(*m);
        assert!(seen.insert(s), "mode_str({m:?}) returned duplicate {s:?}");
    }
    assert_eq!(seen.len(), CompareMode::VARIANTS.len());
}

#[test]
fn classification_label_is_total_and_distinct() {
    let mut seen: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for c in Classification::VARIANTS {
        let s = classification_label(*c);
        assert!(
            seen.insert(s),
            "classification_label({c:?}) duplicate {s:?}"
        );
    }
    assert_eq!(seen.len(), Classification::VARIANTS.len());
}

#[test]
fn classification_slug_is_total_and_distinct() {
    let mut seen: std::collections::BTreeSet<&'static str> = std::collections::BTreeSet::new();
    for c in Classification::VARIANTS {
        let s = classification_slug(*c);
        assert!(seen.insert(s), "classification_slug({c:?}) duplicate {s:?}");
    }
    assert_eq!(seen.len(), Classification::VARIANTS.len());
}
