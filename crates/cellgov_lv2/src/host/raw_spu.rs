//! LV2 dispatch for the Raw SPU calls: create and destroy (160, 161),
//! the interrupt tag, mask and status (150-154), the PU interrupt
//! mailbox (163), `SPU_Cfg` (196, 197) and page-fault recovery (199).
//!
//! A call that takes an interrupt class checks it first (`CELL_EINVAL`
//! for any class but 0 and 2), then the id (`CELL_ESRCH`), then its out
//! pointer (`CELL_EFAULT` when the call cannot write the word there,
//! null included). Create checks the pool before the pointer: `CELL_EAGAIN`
//! once the Raw SPUs that exist take the limit `sys_spu_initialize` set,
//! then `CELL_EFAULT` for an id pointer it cannot write, which creates
//! nothing. Every out word is 32 bits except the mask and the status,
//! which are 64. The spu_raw_lifecycle console capture fixes each
//! answer, each order and each width.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno::{self, Lv2ErrCode};
use cellgov_time::GuestTicks;

use crate::dispatch::Lv2Dispatch;
use crate::host::{Lv2Host, Lv2Runtime};
use crate::raw_spu::RawSpuError;

/// The errno a Raw SPU call answers for `err`.
fn errno_of(err: RawSpuError) -> Lv2ErrCode {
    match err {
        RawSpuError::Unknown => errno::CELL_ESRCH,
        RawSpuError::Exhausted | RawSpuError::TagExists { .. } => errno::CELL_EAGAIN,
        RawSpuError::InvalidClass { .. } | RawSpuError::NoPageFault => errno::CELL_EINVAL,
    }
}

/// `Lv2Dispatch::Immediate` answering `err`.
fn refuse(err: RawSpuError) -> Lv2Dispatch {
    Lv2Dispatch::immediate(errno_of(err).into())
}

/// Whether a call can write a `len`-byte out word at `ptr`.
fn out_writable(rt: &dyn Lv2Runtime, ptr: u32, len: usize) -> bool {
    ptr != 0 && rt.writable(u64::from(ptr), len)
}

/// Where a call writes its out word, and how it stamps the write.
#[derive(Clone, Copy)]
struct Out<'a> {
    rt: &'a dyn Lv2Runtime,
    requester: UnitId,
    tick: GuestTicks,
}

impl Out<'_> {
    /// A write of `bytes` at `ptr`, or `CELL_EFAULT` when they cannot be
    /// written there.
    fn write(self, ptr: u32, bytes: &[u8]) -> Lv2Dispatch {
        if !out_writable(self.rt, ptr, bytes.len()) {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::shared_write(
                ByteRange::contiguous_u32(ptr, bytes.len() as u32),
                WritePayload::from_slice(bytes),
                self.requester,
                self.tick,
            )],
        }
    }
}

impl Lv2Host {
    /// Enter `unit`, which the runtime registered for
    /// [`Lv2Dispatch::RawSpuCreate`], as Raw SPU `id`.
    pub fn record_raw_spu(&mut self, id: u32, unit: UnitId) {
        self.state.raw_spus.insert(id, unit);
    }

    /// The unit that runs Raw SPU `id`, or `None` when no Raw SPU has
    /// the id.
    pub fn raw_spu_unit(&self, id: u32) -> Option<UnitId> {
        self.state.raw_spus.get(id).ok().map(|spu| spu.unit())
    }

    /// `sys_raw_spu_create`
    /// ([`cellgov_ps3_abi::lv2::syscall::RAW_SPU_CREATE`]): choose the
    /// lowest free id and hand the runtime the unit to register.
    pub(in crate::host) fn dispatch_raw_spu_create(
        &self,
        id_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let id = match self.state.raw_spus.free_id(self.state.groups.raw_limit()) {
            Ok(id) => id,
            Err(err) => return refuse(err),
        };
        let out = Out {
            rt,
            requester,
            tick,
        };
        match out.write(id_ptr, &id.to_be_bytes()) {
            Lv2Dispatch::Immediate { code: 0, effects } => {
                Lv2Dispatch::RawSpuCreate { id, effects }
            }
            refusal => refusal,
        }
    }

    /// `sys_raw_spu_destroy`
    /// ([`cellgov_ps3_abi::lv2::syscall::RAW_SPU_DESTROY`]): remove the
    /// Raw SPU with its interrupt tags, and hand the runtime its unit to
    /// finish.
    pub(in crate::host) fn dispatch_raw_spu_destroy(&mut self, id: u32) -> Lv2Dispatch {
        match self.state.raw_spus.remove(id) {
            Ok(spu) => Lv2Dispatch::RawSpuDestroy { unit: spu.unit() },
            Err(err) => refuse(err),
        }
    }

    /// `sys_raw_spu_create_interrupt_tag`
    /// ([`cellgov_ps3_abi::lv2::syscall::RAW_SPU_CREATE_INTERRUPT_TAG`]):
    /// create the tag of `class` and write it as a 32-bit word. The
    /// call ignores the hardware thread, as the kernel does.
    ///
    /// The call refuses a tag pointer it cannot write before it creates
    /// the tag, and so creates none, as the console creates none.
    pub(in crate::host) fn dispatch_raw_spu_create_interrupt_tag(
        &mut self,
        id: u32,
        class: u32,
        tag_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if let Err(err) = self.state.raw_spus.check_class(id, class) {
            return refuse(err);
        }
        if !out_writable(rt, tag_ptr, 4) {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let out = Out {
            rt,
            requester,
            tick,
        };
        match self.state.raw_spus.create_interrupt_tag(id, class) {
            Ok(tag) => out.write(tag_ptr, &tag.to_be_bytes()),
            Err(err) => refuse(err),
        }
    }

    /// `sys_raw_spu_set_int_mask`
    /// ([`cellgov_ps3_abi::lv2::syscall::RAW_SPU_SET_INT_MASK`]).
    pub(in crate::host) fn dispatch_raw_spu_set_int_mask(
        &mut self,
        id: u32,
        class: u32,
        mask: u64,
    ) -> Lv2Dispatch {
        match self.state.raw_spus.set_int_mask(id, class, mask) {
            Ok(()) => Lv2Dispatch::immediate(0),
            Err(err) => refuse(err),
        }
    }

    /// `sys_raw_spu_get_int_mask`
    /// ([`cellgov_ps3_abi::lv2::syscall::RAW_SPU_GET_INT_MASK`]): write
    /// the mask as a 64-bit word.
    pub(in crate::host) fn dispatch_raw_spu_get_int_mask(
        &self,
        id: u32,
        class: u32,
        mask_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let out = Out {
            rt,
            requester,
            tick,
        };
        match self.state.raw_spus.int_mask(id, class) {
            Ok(mask) => out.write(mask_ptr, &mask.to_be_bytes()),
            Err(err) => refuse(err),
        }
    }

    /// `sys_raw_spu_set_int_stat`
    /// ([`cellgov_ps3_abi::lv2::syscall::RAW_SPU_SET_INT_STAT`]): clear
    /// the status bits `stat` sets.
    pub(in crate::host) fn dispatch_raw_spu_set_int_stat(
        &mut self,
        id: u32,
        class: u32,
        stat: u64,
    ) -> Lv2Dispatch {
        match self.state.raw_spus.clear_int_stat(id, class, stat) {
            Ok(()) => Lv2Dispatch::immediate(0),
            Err(err) => refuse(err),
        }
    }

    /// `sys_raw_spu_get_int_stat`
    /// ([`cellgov_ps3_abi::lv2::syscall::RAW_SPU_GET_INT_STAT`]): write
    /// the status as a 64-bit word.
    pub(in crate::host) fn dispatch_raw_spu_get_int_stat(
        &self,
        id: u32,
        class: u32,
        stat_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let out = Out {
            rt,
            requester,
            tick,
        };
        match self.state.raw_spus.int_stat(id, class) {
            Ok(stat) => out.write(stat_ptr, &stat.to_be_bytes()),
            Err(err) => refuse(err),
        }
    }

    /// `sys_raw_spu_read_puint_mb`
    /// ([`cellgov_ps3_abi::lv2::syscall::RAW_SPU_READ_PUINT_MB`]): write
    /// the PU interrupt mailbox word as a 32-bit word. The call answers
    /// `CELL_OK` whether or not the mailbox holds a message.
    pub(in crate::host) fn dispatch_raw_spu_read_puint_mb(
        &self,
        id: u32,
        value_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let out = Out {
            rt,
            requester,
            tick,
        };
        match self.state.raw_spus.puint_mb(id) {
            Ok(word) => out.write(value_ptr, &word.to_be_bytes()),
            Err(err) => refuse(err),
        }
    }

    /// `sys_raw_spu_set_spu_cfg`
    /// ([`cellgov_ps3_abi::lv2::syscall::RAW_SPU_SET_SPU_CFG`]).
    pub(in crate::host) fn dispatch_raw_spu_set_spu_cfg(
        &mut self,
        id: u32,
        value: u32,
    ) -> Lv2Dispatch {
        match self.state.raw_spus.set_cfg(id, value) {
            Ok(()) => Lv2Dispatch::immediate(0),
            Err(err) => refuse(err),
        }
    }

    /// `sys_raw_spu_get_spu_cfg`
    /// ([`cellgov_ps3_abi::lv2::syscall::RAW_SPU_GET_SPU_CFG`]): write
    /// the `SPU_Cfg` word as a 32-bit word.
    pub(in crate::host) fn dispatch_raw_spu_get_spu_cfg(
        &self,
        id: u32,
        value_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        let out = Out {
            rt,
            requester,
            tick,
        };
        match self.state.raw_spus.cfg(id) {
            Ok(cfg) => out.write(value_ptr, &cfg.to_be_bytes()),
            Err(err) => refuse(err),
        }
    }

    /// `sys_raw_spu_recover_page_fault`
    /// ([`cellgov_ps3_abi::lv2::syscall::RAW_SPU_RECOVER_PAGE_FAULT`]).
    pub(in crate::host) fn dispatch_raw_spu_recover_page_fault(&self, id: u32) -> Lv2Dispatch {
        match self.state.raw_spus.recover_page_fault(id) {
            Ok(()) => Lv2Dispatch::immediate(0),
            Err(err) => refuse(err),
        }
    }
}

#[cfg(test)]
#[path = "tests/raw_spu_console_tests.rs"]
mod console_tests;
