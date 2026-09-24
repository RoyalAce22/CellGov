//! The response-update checks and the traced disposition of a request.

use cellgov_event::UnitId;
use cellgov_lv2::PendingResponse;
use cellgov_trace::TracedSyscallDisposition;

use crate::runtime::Runtime;

impl Runtime {
    pub(super) fn assert_response_updates_valid(
        &self,
        site: &'static str,
        woken_unit_ids: &[UnitId],
        response_updates: &[(UnitId, PendingResponse)],
    ) {
        if !cfg!(debug_assertions) {
            return;
        }
        check_response_updates(
            site,
            &self.syscall_responses,
            woken_unit_ids,
            response_updates,
        );
    }
}

/// `Hypercall` and `TimerFastPath` dispositions are set by the caller
/// (it knows LEV; the timer bypass skips classify entirely); every
/// other variant maps here.
pub(super) fn disposition_from_request(
    request: &cellgov_lv2::Lv2Request,
) -> TracedSyscallDisposition {
    match request {
        cellgov_lv2::Lv2Request::NoSuchSyscall { .. } => TracedSyscallDisposition::NoSuchSyscall,
        cellgov_lv2::Lv2Request::Unsupported { .. } => TracedSyscallDisposition::Unsupported,
        cellgov_lv2::Lv2Request::UnresolvedImport { .. } => {
            TracedSyscallDisposition::UnresolvedImport
        }
        cellgov_lv2::Lv2Request::Malformed { .. } => TracedSyscallDisposition::Malformed,
        cellgov_lv2::Lv2Request::Hypercall { .. } => TracedSyscallDisposition::Hypercall,
        _ => TracedSyscallDisposition::Implemented,
    }
}

pub(crate) fn check_response_updates(
    site: &'static str,
    table: &crate::syscall_table::SyscallResponseTable,
    woken_unit_ids: &[UnitId],
    response_updates: &[(UnitId, PendingResponse)],
) {
    for (waiter, update) in response_updates {
        // A legitimate update targets a unit being woken now, or a
        // still-parked unit whose staged response is being replaced --
        // the cond two-hop restages a contended waiter's cond-wait
        // response as the mutex-grant response without waking it.
        // Neither-woken-nor-parked means the update can never deliver.
        assert!(
            woken_unit_ids.contains(waiter) || table.peek(*waiter).is_some(),
            "{site}: response_updates entry for {waiter:?} is neither in woken_unit_ids \
             nor parked with a pending response",
        );
        // ReturnCode is the universal cancel/timeout override and
        // may replace any prior variant; the tag invariant only
        // constrains payload-carrying refinements.
        if matches!(update, PendingResponse::ReturnCode { .. }) {
            continue;
        }
        if let Some(existing) = table.peek(*waiter) {
            // sys_event_flag_cancel refines a parked eflag-wait
            // response into its ECANCELED counterpart; every other
            // payload-carrying update must match the parked variant.
            if matches!(
                (existing, update),
                (
                    PendingResponse::EventFlagWake { .. },
                    PendingResponse::EventFlagCancelWake { .. }
                )
            ) {
                continue;
            }
            assert_eq!(
                existing.variant_tag(),
                update.variant_tag(),
                "{site}: response_updates variant mismatch for {waiter:?} \
                 (existing tag {}, update tag {})",
                existing.variant_tag(),
                update.variant_tag(),
            );
        }
    }
}
