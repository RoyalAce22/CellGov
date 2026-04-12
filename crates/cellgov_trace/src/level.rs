//! `TraceLevel` -- categorical filter for trace records.
//!
//! The brief calls for trace levels so that high-volume categories can
//! be filtered without reworking the writer later. These are *categories*
//! rather than severity levels: a unit-scheduled record is not "more
//! important" than a commit-applied record, but a replay tool may want
//! only commit records, or only state-hash records, depending on what it
//! is verifying.
//!
//! The variants and their `#[repr(u8)]` discriminants are part of the
//! binary trace contract. Do not reorder, do not insert variants in the
//! middle, do not change the explicit discriminant values. New levels
//! must be appended at the end with discriminants strictly greater than
//! [`TraceLevel::Hashes`].

/// Category of a structured trace record.
///
/// Used by the writer to tag records and by the reader (and replay
/// tools) to filter them. Each record produced by the runtime carries
/// exactly one `TraceLevel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[repr(u8)]
pub enum TraceLevel {
    /// Scheduler decisions: unit selected, budget granted, unit blocked,
    /// unit woken, unit finished. The default category for runtime
    /// orchestration records.
    #[default]
    Scheduling = 0,
    /// Effect emission and per-effect lifecycle: an effect was emitted
    /// by a unit, an effect was validated, an effect was rejected.
    Effects = 1,
    /// Commit pipeline activity: batch built, batch staged, batch
    /// applied, batch rolled back due to fault.
    Commits = 2,
    /// State hash checkpoints used for deterministic replay
    /// comparison: committed memory hash, runnable queue hash, sync
    /// object state hash, unit status hash.
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
