//! String mappings shared by both renderers.

use crate::compare::{Classification, CompareMode};

pub(super) fn mode_str(mode: CompareMode) -> &'static str {
    match mode {
        CompareMode::Strict => "strict",
        CompareMode::Memory => "memory",
        CompareMode::Events => "events",
        CompareMode::Prefix => "prefix",
    }
}

pub(super) fn classification_label(c: Classification) -> &'static str {
    match c {
        Classification::Match => "MATCH",
        Classification::Divergence => "DIVERGENCE",
        Classification::Unsupported => "UNSUPPORTED",
        Classification::UnsettledOracle => "UNSETTLED_ORACLE",
    }
}

pub(super) fn classification_slug(c: Classification) -> &'static str {
    match c {
        Classification::Match => "match",
        Classification::Divergence => "divergence",
        Classification::Unsupported => "unsupported",
        Classification::UnsettledOracle => "unsettled_oracle",
    }
}
