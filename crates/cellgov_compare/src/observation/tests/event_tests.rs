//! Pairwise distinctness of ObservedEventKind variants.

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
