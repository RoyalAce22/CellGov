//! Normalized event kinds and per-event records.

use serde::{Deserialize, Serialize};

/// Semantic event kinds for normalized comparison.
///
/// Each runner's adapter coalesces its raw events into these kinds.
/// Timing values are stripped during normalization; only kind, unit,
/// and relative order survive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ObservedEventKind {
    /// A unit sent a mailbox message.
    MailboxSend,
    /// A unit received a mailbox message.
    MailboxReceive,
    /// A DMA transfer completed.
    DmaComplete,
    /// A unit was woken from a blocked state.
    UnitWake,
    /// A unit blocked on a sync primitive.
    UnitBlock,
}

/// A single normalized event in the observation sequence.
///
/// `sequence` is a monotonic index within the observation, not a guest
/// tick: CellGov and RPCS3 have different time models.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObservedEvent {
    /// What happened.
    pub kind: ObservedEventKind,
    /// Which unit was involved.
    pub unit: u64,
    /// Monotonic index within the observation.
    pub sequence: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_kind_variants_are_distinct() {
        let kinds = [
            ObservedEventKind::MailboxSend,
            ObservedEventKind::MailboxReceive,
            ObservedEventKind::DmaComplete,
            ObservedEventKind::UnitWake,
            ObservedEventKind::UnitBlock,
        ];
        for (i, a) in kinds.iter().enumerate() {
            for (j, b) in kinds.iter().enumerate() {
                if i == j {
                    assert_eq!(a, b);
                } else {
                    assert_ne!(a, b);
                }
            }
        }
    }
}
