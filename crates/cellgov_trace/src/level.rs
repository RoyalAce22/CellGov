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
#[path = "tests/level_tests.rs"]
mod tests;
