use cellgov_lv2::GuestBlockReason;

/// Exhaustive match: a new variant fails compilation rather than silently rendering via `Debug`.
pub(in crate::game) fn block_reason_label(reason: &GuestBlockReason) -> String {
    match reason {
        GuestBlockReason::WaitingOnJoin { target } => {
            format!("WaitingOnJoin(target={})", target.raw())
        }
        GuestBlockReason::WaitingOnLwMutex { id } => format!("WaitingOnLwMutex(id={id})"),
        GuestBlockReason::WaitingOnMutex { id } => format!("WaitingOnMutex(id={id})"),
        GuestBlockReason::WaitingOnSemaphore { id } => format!("WaitingOnSemaphore(id={id})"),
        GuestBlockReason::WaitingOnEventQueue { id } => format!("WaitingOnEventQueue(id={id})"),
        GuestBlockReason::WaitingOnEventFlag { id, mask, mode } => {
            format!("WaitingOnEventFlag(id={id}, mask=0x{mask:x}, mode={mode:?})")
        }
        GuestBlockReason::WaitingOnCond { cond_id, mutex_id } => {
            format!("WaitingOnCond(cond={cond_id}, mutex={mutex_id})")
        }
    }
}

#[cfg(test)]
mod tests {
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
}
