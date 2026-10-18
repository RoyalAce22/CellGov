//! LV2's service of an SPU thread's outbound interrupt mailbox: the
//! user event the word asks for, and the answer a send reads back.

use cellgov_event::UnitId;
use cellgov_lv2::SpuThreadError;

use super::Runtime;

impl Runtime {
    /// Serve the word SPU thread `unit` wrote to `SPU_WrOutIntrMbox` in
    /// the step this commit closes, if it wrote one.
    ///
    /// The steps:
    /// 1. The runtime takes the word and the data word in `SPU_WrOutMbox`.
    ///    This wakes a writer parked on either mailbox.
    /// 2. LV2 sends the event.
    /// 3. A send's answer goes to the thread's inbound mailbox.
    ///
    /// A word LV2 refuses stops the thread and records a
    /// [`super::SpuThreadFailure`].
    ///
    /// [CBEA p:134 s:9.5.2] a wrch to SPU_WrOutIntrMbox also sends an interrupt to a processor or other device.
    pub(super) fn serve_spu_thread_interrupt_mailbox(&mut self, unit: UnitId) {
        if self.lv2_host.live_spu_group(unit).is_none() {
            return;
        }
        let Some(spu) = self.registry.get_mut(unit) else {
            return;
        };
        if !spu.take_out_intr_mbox_interrupt() {
            return;
        }
        let word = match self.read_unit_out_intr_mbox(unit) {
            Ok(Some(word)) => word,
            other => {
                return self.lv2_host.log_invariant_break(
                    "runtime.spu_thread_interrupt_mailbox_empty",
                    format_args!(
                        "SPU thread {unit:?} raised its mailbox interrupt and its \
                         SPU_WrOutIntrMbox read {other:?}"
                    ),
                );
            }
        };
        let data3 = self.read_unit_out_mbox(unit).ok().flatten();
        let served = match self.lv2_host.send_spu_thread_user_event(unit, word, data3) {
            Ok(served) => served,
            Err(error) => return self.fail_running_spu_thread(unit, error),
        };
        if let Some(answer) = served.answer {
            if let Err(err) = self.write_unit_in_mbox(unit, answer) {
                self.lv2_host.log_invariant_break(
                    "runtime.spu_user_event_answer_refused",
                    format_args!("SPU thread {unit:?} refused its user-event answer: {err}"),
                );
            }
        }
        if let Some((woken, response)) = served.wake {
            self.timer_wakes.cancel(woken);
            let _ = self.syscall_responses.try_take(woken);
            let _ = self.syscall_responses.insert(woken, response);
            self.resolve_sync_wakes(&[woken]);
            self.step_woke_others = true;
        }
    }

    /// Stop the running SPU thread `unit`, then record its failure with `error`.
    fn fail_running_spu_thread(&mut self, unit: UnitId, error: SpuThreadError) {
        if let Err(err) = self.request_unit_stop(unit) {
            self.lv2_host.log_invariant_break(
                "runtime.spu_user_event_stop_refused",
                format_args!("SPU thread {unit:?} did not stop on its refused event: {err:?}"),
            );
        }
        let Some(stop) = self.unit_stop_registers(unit) else {
            return self.lv2_host.log_invariant_break(
                "runtime.spu_user_event_stop_unreported",
                format_args!("SPU thread {unit:?} reported no stop after its refused event"),
            );
        };
        self.fail_spu_thread(unit, stop, error);
    }
}

#[cfg(test)]
#[path = "tests/spu_user_event_tests.rs"]
mod tests;
