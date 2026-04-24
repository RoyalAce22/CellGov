//! Categorical filter tag attached to every trace record.
//!
//! Variants and their `#[repr(u8)]` discriminants are part of the binary trace
//! contract: do not reorder, do not insert in the middle, do not change values.
//! New levels append with discriminants strictly greater than
//! [`TraceLevel::Hashes`].

/// Category of a structured trace record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum TraceLevel {
    /// Scheduler decisions: select, grant, block, wake, finish.
    #[default]
    Scheduling = 0,
    /// Effect emission and per-effect lifecycle.
    Effects = 1,
    /// Commit pipeline activity.
    Commits = 2,
    /// State-hash checkpoints used for replay comparison.
    Hashes = 3,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discriminants_are_locked() {
        assert_eq!(TraceLevel::Scheduling as u8, 0);
        assert_eq!(TraceLevel::Effects as u8, 1);
        assert_eq!(TraceLevel::Commits as u8, 2);
        assert_eq!(TraceLevel::Hashes as u8, 3);
    }

    #[test]
    fn variants_are_distinct() {
        let all = [
            TraceLevel::Scheduling,
            TraceLevel::Effects,
            TraceLevel::Commits,
            TraceLevel::Hashes,
        ];
        let unique: std::collections::BTreeSet<u8> = all.iter().map(|l| *l as u8).collect();
        assert_eq!(unique.len(), all.len());
    }

    #[test]
    fn default_is_scheduling() {
        assert_eq!(TraceLevel::default(), TraceLevel::Scheduling);
    }

    #[test]
    fn equality_distinguishes() {
        assert_eq!(TraceLevel::Effects, TraceLevel::Effects);
        assert_ne!(TraceLevel::Effects, TraceLevel::Commits);
    }
}
