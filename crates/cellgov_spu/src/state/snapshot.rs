//! The observable snapshots an instruction comparison reads.

use cellgov_sync::ReservedLine;

use super::{
    ChannelState, ListCursor, SignalNotifyRegister, SpuState, TagUpdateCondition, SPU_REG_COUNT,
};
use crate::stop::SpuStop;

/// Architectural state for instruction comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuObservableSnapshot {
    /// Architectural register file.
    pub regs: [[u8; 16]; SPU_REG_COUNT],
    /// Architectural local store.
    pub ls: Vec<u8>,
    /// Address of the next instruction.
    pub pc: u32,
    /// Local storage limit register.
    pub lslr: u32,
    /// Signal-notification registers.
    pub signals: [SignalNotifyRegister; 2],
    /// Architectural channel state.
    pub channels: SpuChannelSnapshot,
    /// The facilities a multi-instruction run reaches.
    pub facilities: SpuFacilitySnapshot,
    /// Atomic reservation state.
    pub reservation: Option<ReservedLine>,
    /// Stopped state.
    pub stop: Option<SpuStop>,
    /// Floating-point status and control register.
    pub fpscr: u128,
    /// Interrupt-enable state.
    pub interrupts_enabled: bool,
    /// State save and restore register 0.
    pub srr0: u32,
}

/// The SPU facilities a multi-instruction run reaches, each compared on its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuFacilitySnapshot {
    /// The event registers.
    pub events: SpuEventSnapshot,
    /// The decrementer.
    pub decrementer: crate::decrementer::Decrementer,
    /// The message in the outbound interrupt mailbox.
    pub out_intr_mbox: Option<u32>,
    /// The list commands still queuing elements.
    pub lists: SpuListSnapshot,
    /// The queue positions the tracked request and a new request wait for, in that order.
    pub mssync: (Option<u64>, Option<u64>),
}

/// The event registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpuEventSnapshot {
    /// The pending-event register.
    pub pending: u32,
    /// The event mask.
    pub mask: u32,
    /// The `SPU_RdEventStat` count.
    pub count: bool,
    /// The event-source levels the last event update found.
    pub levels: u32,
}

/// The list commands still queuing elements.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuListSnapshot {
    /// Where each list resumes.
    pub cursors: Vec<ListCursor>,
    /// The `MFC_RdListStallStat` groups.
    pub stall_status: u32,
}

/// Channel state for instruction comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpuChannelSnapshot {
    /// Address staged for the next MFC command.
    pub mfc_lsa: u32,
    /// High effective-address word for the next MFC command.
    pub mfc_eah: u32,
    /// Low effective-address word for the next MFC command.
    pub mfc_eal: u32,
    /// Transfer size for the next MFC command.
    pub mfc_size: u32,
    /// Tag identifier for the next MFC command.
    pub mfc_tag_id: u32,
    /// Mask for tag-status queries.
    pub tag_mask: u32,
    /// Tag-status word as the last step entry built it: a set bit means
    /// that group had no outstanding transfer then. A new context holds 0.
    pub tag_status: u32,
    /// Status of the last atomic command.
    pub atomic_status: u32,
    /// Free slots in the MFC command queue.
    pub cmd_queue_free: u32,
    /// A waiting conditional tag-status update request.
    pub tag_update: Option<TagUpdateCondition>,
    /// The `MFC_RdTagStat` data of a met update request, not yet read.
    pub tag_status_read: Option<u32>,
    /// An atomic command's status is waiting to be read.
    pub atomic_status_ready: bool,
    /// Messages in the inbound mailbox, oldest first, less the ones the
    /// step has read.
    pub in_mbox: Vec<u32>,
    /// The message in the outbound mailbox.
    pub out_mbox: Option<u32>,
}

impl SpuObservableSnapshot {
    /// Creates an instruction-comparison snapshot.
    pub fn capture(state: &SpuState) -> Self {
        Self::with_local_store(state, state.ls.clone())
    }

    /// Creates the snapshot of a state no caller reads again, and keeps
    /// its local store without a copy.
    pub fn capture_owned(mut state: SpuState) -> Self {
        let ls = std::mem::take(&mut state.ls);
        Self::with_local_store(&state, ls)
    }

    /// The snapshot of every component except the local store, which is
    /// left empty. Comparisons read the local store in place instead.
    pub(crate) fn capture_without_local_store(state: &SpuState) -> Self {
        Self::with_local_store(state, Vec::new())
    }

    /// Whether this snapshot equals [`Self::capture`] of `state`, without
    /// the copy of the local store that capture makes.
    pub fn matches(&self, state: &SpuState) -> bool {
        self.ls == state.ls && self.eq_except_local_store(&Self::capture_without_local_store(state))
    }

    /// Whether every component except the local store is equal.
    fn eq_except_local_store(&self, other: &Self) -> bool {
        let Self {
            regs,
            ls: _,
            pc,
            lslr,
            signals,
            channels,
            facilities,
            reservation,
            stop,
            fpscr,
            interrupts_enabled,
            srr0,
        } = self;
        *regs == other.regs
            && *pc == other.pc
            && *lslr == other.lslr
            && *signals == other.signals
            && *channels == other.channels
            && *facilities == other.facilities
            && *reservation == other.reservation
            && *stop == other.stop
            && *fpscr == other.fpscr
            && *interrupts_enabled == other.interrupts_enabled
            && *srr0 == other.srr0
    }

    fn with_local_store(state: &SpuState, ls: Vec<u8>) -> Self {
        let SpuState {
            regs,
            // The caller supplies the local store.
            ls: _,
            pc,
            lslr,
            signals,
            channels,
            reservation,
            stop,
            fpscr,
            interrupts_enabled,
            srr0,
            acc: _,
        } = state;
        let ChannelState {
            mfc_lsa,
            mfc_eah,
            mfc_eal,
            mfc_size,
            mfc_tag_id,
            tag_mask,
            tag_status,
            atomic_status,
            cmd_queue_free,
            tag_update,
            tag_status_read,
            atomic_status_ready,
            in_mbox,
            out_mbox,
            // The SPU's setup fixes whether a reader is connected, and
            // the reader takes the interrupt edge.
            out_intr_mbox_connected: _,
            out_intr_mbox,
            out_intr_mbox_interrupt: _,
            lists,
            // The step entry copies it from the queue, as it builds
            // `tag_status` from the queue.
            mfc_ordering: _,
            list_stall_status,
            mssync_tracking,
            mssync_horizon,
            pending_events,
            event_mask,
            event_count,
            event_levels,
            decrementer,
            // The world sets the clock for the next instruction, so the
            // snapshot omits it.
            clock: _,
            // A step that read the clock tells the explorer through a
            // `ClockRead` effect.
            clock_read: _,
        } = channels;
        Self {
            regs: *regs.as_array(),
            ls,
            pc: *pc,
            lslr: *lslr,
            signals: *signals,
            channels: SpuChannelSnapshot {
                mfc_lsa: *mfc_lsa,
                mfc_eah: *mfc_eah,
                mfc_eal: *mfc_eal,
                mfc_size: *mfc_size,
                mfc_tag_id: *mfc_tag_id,
                tag_mask: *tag_mask,
                tag_status: *tag_status,
                atomic_status: *atomic_status,
                cmd_queue_free: *cmd_queue_free,
                tag_update: *tag_update,
                tag_status_read: *tag_status_read,
                atomic_status_ready: *atomic_status_ready,
                in_mbox: in_mbox.clone(),
                out_mbox: *out_mbox,
            },
            facilities: SpuFacilitySnapshot {
                events: SpuEventSnapshot {
                    pending: *pending_events,
                    mask: *event_mask,
                    count: *event_count,
                    levels: *event_levels,
                },
                decrementer: *decrementer,
                out_intr_mbox: *out_intr_mbox,
                lists: SpuListSnapshot {
                    cursors: lists.clone(),
                    stall_status: *list_stall_status,
                },
                mssync: (*mssync_tracking, *mssync_horizon),
            },
            reservation: *reservation,
            stop: *stop,
            fpscr: *fpscr,
            interrupts_enabled: *interrupts_enabled,
            srr0: *srr0,
        }
    }
}
