//! Per-thread channels and configuration: the inbound mailbox, the
//! signal-notification registers, and the signal configuration.

use cellgov_effects::{Effect, MailboxMessage, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::spu;
use cellgov_sync::MailboxId;
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::Lv2Host;

impl Lv2Host {
    /// `sys_spu_thread_write_spu_mb`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_WRITE_MB`]):
    /// deposit `value` in the inbound mailbox of the running SPU that
    /// `thread_id` names.
    ///
    /// # Errors
    ///
    /// - `CELL_ESRCH` when no running SPU carries `thread_id`.
    pub(in crate::host) fn dispatch_write_mb(
        &self,
        thread_id: u32,
        value: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let target_uid = match self.state.groups.running_unit_for_thread(thread_id) {
            Some(uid) => uid,
            None => {
                return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
            }
        };
        let effect = Effect::MailboxSend {
            mailbox: MailboxId::new(target_uid.raw()),
            message: MailboxMessage::new(value),
            source: requester,
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![effect],
        }
    }

    /// `sys_spu_thread_write_snr`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_WRITE_SNR`]): write
    /// signal-notification register `number + 1` of the thread
    /// `thread_id` names. A running SPU takes the value in the mode its
    /// live configuration gives that register. A thread with no running
    /// SPU, before its group starts or after it ends, has the value kept
    /// in the mode its kept configuration gives, and its next start loads
    /// it; the `spu_snr_outside_run` capture shows both runs reading the
    /// kept values.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` when `number` is neither 0 nor 1. This check
    ///   comes first, as the `spu_signal_mbox_edges` capture shows.
    /// - `CELL_ESRCH` when no initialized thread has `thread_id`.
    pub(in crate::host) fn dispatch_write_snr(
        &mut self,
        thread_id: u32,
        number: u32,
        value: u32,
        requester: UnitId,
    ) -> Lv2Dispatch {
        let Ok(register @ 0..=1) = u8::try_from(number) else {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        };
        let Some(target) = self.state.groups.running_unit_for_thread(thread_id) else {
            if self.state.groups.save_signal(thread_id, register, value) {
                return Lv2Dispatch::immediate(0);
            }
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::SpuSignalWrite {
                target,
                register,
                value,
                source: requester,
            }],
        }
    }

    /// `sys_spu_thread_set_spu_cfg`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_SET_SPU_CFG`]): set
    /// the signal configuration of an initialized thread. Bit 0 puts
    /// signal-notification register 1 in OR mode, bit 1 register 2.
    /// While the thread's group runs, the value holds for that run only:
    /// the `spu_signal_mbox_edges` capture reads it back during the run
    /// and the configuration set before the start after it.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` when `value` sets a bit above the two register
    ///   modes. This check comes first, as the `spu_signal_mbox_edges`
    ///   capture shows.
    /// - `CELL_ESRCH` when no initialized thread has `thread_id`.
    pub(in crate::host) fn dispatch_set_spu_cfg(
        &mut self,
        thread_id: u32,
        value: u64,
    ) -> Lv2Dispatch {
        if value > spu::SPU_CFG_SIGNAL_MODE_BITS {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if !self.state.groups.set_signal_config(thread_id, value) {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        Lv2Dispatch::immediate(0)
    }

    /// `sys_spu_thread_get_spu_cfg`
    /// ([`cellgov_ps3_abi::lv2::syscall::SPU_THREAD_GET_SPU_CFG`]): store
    /// the signal configuration of an initialized thread at `value_ptr`
    /// as a u64: the running SPU's while its group runs, the kept one
    /// otherwise.
    ///
    /// # Errors
    ///
    /// As the `spu_signal_mbox_edges` capture shows:
    ///
    /// - `CELL_ESRCH` when no initialized thread has `thread_id`. This
    ///   check comes first.
    /// - `CELL_EFAULT` when `value_ptr` is null.
    pub(in crate::host) fn dispatch_get_spu_cfg(
        &self,
        thread_id: u32,
        value_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let Some(config) = self.state.groups.signal_config(thread_id) else {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if let Some(refusal) = self.efault_if_null(&[value_ptr]) {
            return refusal;
        }
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::shared_write(
                ByteRange::contiguous_u32(value_ptr, 8),
                WritePayload::from_slice(&config.to_be_bytes()),
                requester,
                tick,
            )],
        }
    }
}
