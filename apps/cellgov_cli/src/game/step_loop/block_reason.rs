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
#[path = "tests/block_reason_tests.rs"]
mod tests;
