//! Guest block-reason label uniqueness and field rendering.

use super::*;

#[test]
fn block_reason_label_distinguishes_each_variant() {
    use cellgov_lv2::{EventFlagWaitMode, PpuThreadId};
    let labels = [
        block_reason_label(&GuestBlockReason::WaitingOnJoin {
            target: PpuThreadId::PRIMARY,
        }),
        block_reason_label(&GuestBlockReason::WaitingOnLwMutex { id: 7 }),
        block_reason_label(&GuestBlockReason::WaitingOnMutex { id: 7 }),
        block_reason_label(&GuestBlockReason::WaitingOnSemaphore { id: 7 }),
        block_reason_label(&GuestBlockReason::WaitingOnEventQueue { id: 7 }),
        block_reason_label(&GuestBlockReason::WaitingOnEventFlag {
            id: 7,
            mask: 0xF0,
            mode: EventFlagWaitMode::AndClear,
        }),
        block_reason_label(&GuestBlockReason::WaitingOnCond {
            cond_id: 11,
            mutex_id: 13,
        }),
    ];
    let unique: std::collections::BTreeSet<_> = labels.iter().collect();
    assert_eq!(unique.len(), labels.len(), "label collision: {labels:?}",);
    assert!(labels[1].contains("id=7"), "got {}", labels[1]);
    assert!(labels[5].contains("mask=0xf0"), "got {}", labels[5]);
    assert!(labels[6].contains("cond=11"), "got {}", labels[6]);
    assert!(labels[6].contains("mutex=13"), "got {}", labels[6]);
}
