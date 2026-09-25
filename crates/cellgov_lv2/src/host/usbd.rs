//! `sys_usbd` (530-541): the USB host driver as the guest sees it --
//! driver handles and the event wait -- on a bus with no device.
//!
//! Nothing ever attaches: an event reader parks until
//! `sys_usbd_finalize` wakes every reader with the terminate triple,
//! and every device- or pipe-scoped call answers the refusal an empty
//! bus gives.
//!
//! `libusbd.sprx` is the firmware side of this surface: it issues 530
//! through 540 from its own wrappers, and the audio server reaches the
//! same set. Nothing in dev_flash issues 541. No firmware witness
//! fixes what any of them answer on an empty bus. The refusals below
//! are the model's own, and the kernel's real answers are
//! unestablished.

use std::collections::{BTreeSet, VecDeque};

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::usbd as usb;
use cellgov_time::GuestTicks;

use crate::dispatch::{Lv2BlockReason, Lv2Dispatch, PendingResponse};
use crate::host::{Lv2Host, Lv2Runtime};
use crate::ppu_thread::PpuThreadId;

/// An event reader parked on a driver handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UsbdWaiter {
    pub thread: PpuThreadId,
    pub handle: u32,
    /// `arg1`, `arg2`, `arg3` out pointers, each a u64.
    pub out_ptrs: [u32; 3],
}

/// Lane layout of the USB driver:
///
/// - Field 1 is the handle count.
/// - Field 2 is handle `i`, at slot `i`.
/// - Field 3 is the product-string count.
/// - Field 4 is product string `i`, under key `i`.
/// - Field 5 is the reader count.
/// - Fields 6 to 10 are parked reader `i`, at slot `i`.
///
/// The exhaustive destructure makes a new field without a lane a
/// compile error.
impl cellgov_mem::lanes::LaneValue for UsbdState {
    fn lanes(&self, lanes: &mut cellgov_mem::lanes::ObjectLanes) {
        let Self {
            handles,
            ldds,
            waiters,
        } = self;
        lanes.lane(1, 0, handles.len() as u64);
        for (slot, handle) in handles.iter().enumerate() {
            lanes.lane(2, slot as u64, u64::from(*handle));
        }
        lanes.lane(3, 0, ldds.len() as u64);
        for (slot, product) in ldds.iter().enumerate() {
            lanes.bytes(4, &[slot as u64], product);
        }
        lanes.lane(5, 0, waiters.len() as u64);
        for (slot, w) in waiters.iter().enumerate() {
            let slot = slot as u64;
            lanes.lane(6, slot, w.thread.raw());
            lanes.lane(7, slot, u64::from(w.handle));
            for (i, ptr) in w.out_ptrs.iter().enumerate() {
                lanes.lane(8 + i as u8, slot, u64::from(*ptr));
            }
        }
    }
}

/// Driver handles, registered logical device drivers, and parked
/// event readers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UsbdState {
    handles: BTreeSet<u32>,
    /// Product strings `sys_usbd_register_ldd` recorded; unregister
    /// answers ESRCH for one that is absent.
    ldds: BTreeSet<Vec<u8>>,
    /// Parked readers in park order; finalize wakes them in this
    /// order.
    waiters: VecDeque<UsbdWaiter>,
}

impl UsbdState {
    pub(crate) fn new() -> Self {
        Self {
            handles: BTreeSet::new(),
            ldds: BTreeSet::new(),
            waiters: VecDeque::new(),
        }
    }

    #[cfg(test)]
    pub(crate) fn ldds(&self) -> &BTreeSet<Vec<u8>> {
        &self.ldds
    }

    /// True until `sys_usbd_initialize`.
    #[cfg(test)]
    pub(crate) fn is_pristine(&self) -> bool {
        *self == Self::new()
    }

    /// The driver's term of the sync-state sum, computed on read.
    pub(crate) fn sync_term(&self) -> u128 {
        cellgov_mem::lanes::value_term(cellgov_mem::lanes::source::USBD, 0, self)
    }

    #[cfg(test)]
    pub(crate) fn handles(&self) -> &BTreeSet<u32> {
        &self.handles
    }

    #[cfg(test)]
    pub(crate) fn waiters(&self) -> &VecDeque<UsbdWaiter> {
        &self.waiters
    }

    /// Remove every parked reader whose thread is in `threads`,
    /// preserving the order of survivors; returns the removed
    /// records. Process-exit purge.
    #[must_use = "the purged readers are the only witness that these wakes were cancelled"]
    pub(crate) fn purge_waiters_of(&mut self, threads: &BTreeSet<PpuThreadId>) -> Vec<UsbdWaiter> {
        let mut removed = Vec::new();
        self.waiters.retain(|w| {
            if threads.contains(&w.thread) {
                removed.push(*w);
                false
            } else {
                true
            }
        });
        removed
    }
}

impl Lv2Host {
    /// `sys_usbd_initialize` (530): mints a driver handle. Every call
    /// mints a fresh one and all of them stay live; whether the kernel
    /// caps the count is unestablished.
    ///
    /// # Errors
    ///
    /// - `CELL_EFAULT` for a null `handle_ptr`.
    pub(super) fn dispatch_usbd_initialize(
        &mut self,
        handle_ptr: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if let Some(d) = self.efault_if_null(&[handle_ptr]) {
            return d;
        }
        let handle = self.alloc_id();
        self.state.usbd.handles.insert(handle);
        self.immediate_write_u32(handle, handle_ptr, requester, tick)
    }

    /// `sys_usbd_finalize` (531): drops the handle and wakes every
    /// parked event reader with `(SYS_USBD_TERMINATE, 0, 0)`, in park
    /// order.
    ///
    /// Readers parked on another handle wake too. Park order is
    /// CellGov's own; the kernel's drain order has no witness here.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for a handle no initialize minted.
    pub(super) fn dispatch_usbd_finalize(
        &mut self,
        handle: u32,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if !self.state.usbd.handles.remove(&handle) {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let waiters: Vec<UsbdWaiter> = self.state.usbd.waiters.drain(..).collect();
        if waiters.is_empty() {
            return Lv2Dispatch::immediate(0);
        }
        let mut woken_unit_ids = Vec::with_capacity(waiters.len());
        let mut response_updates = Vec::with_capacity(waiters.len());
        let mut effects = Vec::with_capacity(waiters.len() * 3);
        for w in waiters {
            let Some(unit) = self.resolve_wake_thread(w.thread, "usbd_finalize.waiter") else {
                continue;
            };
            woken_unit_ids.push(unit);
            response_updates.push((unit, PendingResponse::ReturnCode { code: 0 }));
            for (ptr, value) in w.out_ptrs.into_iter().zip([usb::SYS_USBD_TERMINATE, 0, 0]) {
                effects.push(Effect::shared_write(
                    ByteRange::contiguous_u32(ptr, 8),
                    WritePayload::from_slice(&value.to_be_bytes()),
                    requester,
                    tick,
                ));
            }
        }
        if woken_unit_ids.is_empty() {
            return Lv2Dispatch::immediate(0);
        }
        Lv2Dispatch::WakeAndReturn {
            code: 0,
            woken_unit_ids,
            response_updates,
            effects,
        }
    }

    /// `sys_usbd_get_device_list` (532): the attached-device count,
    /// which is always 0 here, so nothing is written.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for a handle no initialize minted.
    pub(super) fn dispatch_usbd_get_device_list(&mut self, handle: u32) -> Lv2Dispatch {
        if !self.state.usbd.handles.contains(&handle) {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        Lv2Dispatch::immediate(0)
    }

    /// Read the product string an LDD call names; `Err` carries the
    /// arm's answer.
    fn usbd_product(
        &self,
        handle: u32,
        product_ptr: u32,
        product_len: u32,
        rt: &dyn Lv2Runtime,
    ) -> Result<Vec<u8>, errno::Lv2ErrCode> {
        if !self.state.usbd.handles.contains(&handle) {
            return Err(errno::CELL_EINVAL);
        }
        // The kernel signature carries the length as a u16.
        let len = usize::from(product_len as u16);
        if len == 0 {
            return Ok(Vec::new());
        }
        rt.read_committed(u64::from(product_ptr), len)
            .map(<[u8]>::to_vec)
            .ok_or(errno::CELL_EFAULT)
    }

    /// `sys_usbd_register_ldd` (535): records the product string so a
    /// later unregister can find it. Registering a product twice is
    /// acknowledged like the first time. No device ever matches it
    /// here, so the record is only ever read by
    /// `sys_usbd_unregister_ldd`.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for a handle no initialize minted.
    /// - `CELL_EFAULT` for an unreadable product string.
    pub(super) fn dispatch_usbd_register_ldd(
        &mut self,
        handle: u32,
        product_ptr: u32,
        product_len: u32,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let product = match self.usbd_product(handle, product_ptr, product_len, rt) {
            Ok(p) => p,
            Err(code) => return Lv2Dispatch::immediate(code.into()),
        };
        self.state.usbd.ldds.insert(product);
        Lv2Dispatch::immediate(0)
    }

    /// `sys_usbd_unregister_ldd` (536): forgets a product
    /// `sys_usbd_register_ldd` recorded.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for a handle no initialize minted.
    /// - `CELL_EFAULT` for an unreadable product string.
    /// - `CELL_ESRCH` for a product no register recorded.
    pub(super) fn dispatch_usbd_unregister_ldd(
        &mut self,
        handle: u32,
        product_ptr: u32,
        product_len: u32,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        let product = match self.usbd_product(handle, product_ptr, product_len, rt) {
            Ok(p) => p,
            Err(code) => return Lv2Dispatch::immediate(code.into()),
        };
        if !self.state.usbd.ldds.remove(&product) {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        Lv2Dispatch::immediate(0)
    }

    /// `sys_usbd_get_descriptor` (534): a null descriptor pointer is
    /// an argument fault, so it is refused ahead of the device gate.
    /// It stays out of `usbd_no_device_refusals`, which counts only
    /// the empty bus.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` always: an unminted handle, a null descriptor
    ///   pointer, or the empty bus.
    pub(super) fn dispatch_usbd_get_descriptor(
        &mut self,
        handle: u32,
        desc_ptr: u32,
    ) -> Lv2Dispatch {
        if !self.state.usbd.handles.contains(&handle) || desc_ptr == 0 {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        self.dispatch_usbd_no_device(handle)
    }

    /// The empty-bus refusal the device- and pipe-scoped arms share:
    ///
    /// - `sys_usbd_get_descriptor_size` (533)
    /// - the device gate of `sys_usbd_get_descriptor` (534)
    /// - `sys_usbd_open_pipe` (537)
    /// - `sys_usbd_open_default_pipe` (538)
    /// - `sys_usbd_close_pipe` (539)
    ///
    /// No device ever attaches, so no device or pipe handle exists and
    /// every one answers `CELL_EINVAL`. Counted in
    /// `usbd_no_device_refusals`.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` always; the handle gate fires first.
    pub(super) fn dispatch_usbd_no_device(&mut self, handle: u32) -> Lv2Dispatch {
        if self.state.usbd.handles.contains(&handle) {
            self.obs.usbd_no_device_refusals += 1;
        }
        Lv2Dispatch::immediate(errno::CELL_EINVAL.into())
    }

    /// `sys_usbd_receive_event` (540): parks the caller until
    /// `sys_usbd_finalize`; an empty bus never queues an event.
    ///
    /// # Errors
    ///
    /// - `CELL_EINVAL` for a handle no initialize minted.
    /// - `CELL_EFAULT` for a null or unwritable out pointer.
    /// - `CELL_ESRCH` for a caller with no PPU thread record.
    pub(super) fn dispatch_usbd_receive_event(
        &mut self,
        handle: u32,
        out_ptrs: [u32; 3],
        requester: UnitId,
        rt: &dyn Lv2Runtime,
    ) -> Lv2Dispatch {
        if !self.state.usbd.handles.contains(&handle) {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if let Some(d) = self.efault_if_null(&out_ptrs) {
            return d;
        }
        if out_ptrs.iter().any(|&p| !rt.writable(u64::from(p), 8)) {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let Some(thread) = self.state.ppu_threads.thread_id_for_unit(requester) else {
            self.log_invariant_break(
                "dispatch.usbd_receive_caller_without_thread_record",
                format_args!("sys_usbd_receive_event: unit {requester:?} has no PPU thread record"),
            );
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        };
        if self.state.usbd.waiters.iter().any(|w| w.thread == thread) {
            // A parked thread cannot dispatch; two records for one
            // thread would wake it twice.
            self.log_invariant_break(
                "dispatch.usbd_reader_reparked",
                format_args!(
                    "sys_usbd_receive_event: {thread:?} is already parked; returning CELL_ESRCH"
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        self.state.usbd.waiters.push_back(UsbdWaiter {
            thread,
            handle,
            out_ptrs,
        });
        Lv2Dispatch::Block {
            reason: Lv2BlockReason::UsbdEvent { handle },
            pending: PendingResponse::ReturnCode { code: 0 },
            effects: vec![],
        }
    }

    /// `sys_usbd_detect_event` (541): answers `CELL_OK` and models
    /// nothing, so each call records an invariant break. No dev_flash
    /// module issues it.
    pub(super) fn dispatch_usbd_detect_event(&mut self) -> Lv2Dispatch {
        self.log_invariant_break(
            "dispatch.usbd_detect_event_unmodeled",
            format_args!("sys_usbd_detect_event acknowledged with CELL_OK; nothing is modeled"),
        );
        Lv2Dispatch::immediate(0)
    }
}

#[cfg(test)]
#[path = "tests/usbd_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/usbd_lanes_tests.rs"]
mod lanes_tests;
