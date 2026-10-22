//! The channel and MFC queue state.

use cellgov_ps3_abi::hw::spu::{MFC_TAG_UPDATE_ALL, MFC_TAG_UPDATE_ANY, MFC_TAG_UPDATE_IMMEDIATE};

/// MFC and channel state read/written by rdch/wrch/rchcnt.
///
/// The MFC command parameter channels keep their last-written values
/// after the SPU enqueues a command, except `MFC_EAH`, which returns to 0. A
/// program that does not rewrite a required parameter issues a command
/// with the previous one's value; the architecture calls that value
/// invalid and leaves what the channel holds unstated.
/// [CBEA p:121 s:9.2] after a command is queued the parameter values become invalid, and omitting a required parameter can make the queue operate improperly.
/// [CBEA p:52 s:7] when EAH is not specified on a command, hardware must set EAH to '0'.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelState {
    /// MFC_LSA: local store address for next DMA command.
    ///
    /// [CBEA p:110 s:9] MFC_LSA channel x'10': local storage address command parameter.
    pub mfc_lsa: u32,
    /// MFC_EAH: effective address high word.
    ///
    /// [CBEA p:110 s:9] MFC_EAH channel x'11': high-order EA command parameter.
    pub mfc_eah: u32,
    /// MFC_EAL: effective address low word.
    ///
    /// [CBEA p:111 s:9] MFC_EAL channel x'12': low-order EA / list address command parameter.
    pub mfc_eal: u32,
    /// MFC_Size: transfer size for next DMA command.
    ///
    /// [CBEA p:111 s:9] MFC_Size channel x'13': transfer size / list size command parameter.
    pub mfc_size: u32,
    /// MFC_TagID: tag for next DMA command.
    ///
    /// [CBEA p:111 s:9] MFC_TagID channel x'14': tag identifier command parameter.
    pub mfc_tag_id: u32,
    /// Tag mask written by mfc_write_tag_mask.
    ///
    /// [CBEA p:111 s:9] MFC_WrTagMask channel x'16': tag-group query mask.
    pub tag_mask: u32,
    /// Tag groups with no outstanding transfer, one bit per group,
    /// rebuilt at the start of each `run_until_yield`. A new context
    /// holds 0 until its first step.
    ///
    /// [CBEA p:111 s:9] MFC_RdTagStat channel x'18': tag-group status bits.
    pub tag_status: u32,
    /// Atomic operation status set after getllar/putllc.
    ///
    /// [CBEA p:111 s:9] MFC_RdAtomicStat channel x'1B': atomic-command completion status.
    pub atomic_status: u32,
    /// Free slots in the MFC command queue: the queue depth less the
    /// unit's commands queued and not yet complete, which the runtime
    /// reports at the start of each step. It is the `MFC_Cmd` count, and
    /// a put or get the step enqueues takes one.
    ///
    /// A count that rises from 0 at a step's start is where the Qv event
    /// comes from: a slot freed while the queue was full.
    ///
    /// [CBEA p:113 s:9.1.1] the MFC_Cmd count is the number of free command-queue slots.
    /// [CBEA p:159 s:9.12.3] a slot freeing when the queue was full raises the Qv event.
    pub cmd_queue_free: u32,
    /// A waiting conditional tag-status update request.
    ///
    /// [CBEA p:127 s:9.3.5] an update request updates the status immediately, when any enabled group completes, or when all enabled groups complete.
    pub tag_update: Option<TagUpdateCondition>,
    /// The `MFC_RdTagStat` data a met update request latched.
    ///
    /// `Some` is a channel count of 1. A read takes the data.
    ///
    /// [CBEA p:128 s:9.3.6] the channel holds the status of the groups enabled at the time of the last update; its count turns 1 when that status is available.
    pub tag_status_read: Option<u32>,
    /// An atomic command completed and `MFC_RdAtomicStat` has not been
    /// read since, so the channel counts 1.
    ///
    /// [CBEA p:131 s:9.4] the MFC_RdAtomicStat count starts at 0 and is 1 once an immediate atomic command completes.
    pub atomic_status_ready: bool,
    /// The messages in the unit's inbound mailbox, oldest first, as the
    /// runtime reported them at the start of the step, less the ones
    /// the step has read. Its length is the `SPU_RdInMbox` count.
    ///
    /// [CBEA p:111 s:9] SPU_RdInMbox channel x'1D': PPE-to-SPU mailbox read.
    /// [CBEA p:135 s:9.5.3] the SPU_RdInMbox count is the number of messages in the inbound mailbox and starts at 0.
    pub in_mbox: Vec<u32>,
    /// The message the SPU wrote to `SPU_WrOutMbox` and no processor
    /// has read. The mailbox holds one.
    ///
    /// [CBEA p:98 s:8.6.1] an MMIO read of SPU_Out_Mbox returns the messages in the order the SPU wrote them.
    /// [CBE-Handbook p:445 s:17.1 Table 17-2] SPU_WrOutMbox has 1 maximum entry.
    pub out_mbox: Option<u32>,
    /// Whether a reader outside the unit empties `SPU_WrOutIntrMbox`, as
    /// for a Raw SPU. A unit without one refuses the channel by name.
    pub out_intr_mbox_connected: bool,
    /// The unread message the SPU wrote to `SPU_WrOutIntrMbox`. The
    /// mailbox holds one message.
    ///
    /// [CBEA p:134 s:9.5.2] a write to a full outbound interrupt mailbox stalls the SPU until the SPU Outbound Interrupt Mailbox Register is read.
    pub out_intr_mbox: Option<u32>,
    /// A write to `SPU_WrOutIntrMbox` sent an interrupt, and the
    /// interrupt controller did not take it yet.
    ///
    /// [CBEA p:134 s:9.5.2] a write to the channel also sends an interrupt to a processor or other device.
    pub out_intr_mbox_interrupt: bool,
    /// The unfinished list commands, oldest first.
    ///
    /// Each list here is one of:
    ///
    /// - stopped at a stall-and-notify element;
    /// - held behind the commands its ordering names.
    ///
    /// Each holds its command-queue slot and its tag group until it queues
    /// its last element.
    pub lists: Vec<ListCursor>,
    /// The unit's queued commands that a new command can wait behind, as
    /// the step began.
    pub mfc_ordering: cellgov_exec::MfcOrderingView,
    /// Tag groups whose list stalled since the last read of
    /// `MFC_RdListStallStat`, one bit per group. A nonzero value is a
    /// channel count of 1.
    ///
    /// [CBEA p:129 s:9.3.7] the channel reports the tag groups with a stalled list; a read clears it and sets the count to 0.
    pub list_stall_status: u32,
    /// The DMA queue sequence a multisource synchronization request
    /// tracks up to: the request completes once no transfer to or from
    /// the unit's local store that the queue took before that sequence
    /// is outstanding. `None` is a channel count of 1.
    ///
    /// [CBEA p:143 s:9.10] a write of MFC_WrMSSyncReq starts tracking the transfers outstanding to the MFC, and the count returns to 1 when they complete.
    pub mssync_tracking: Option<u64>,
    /// The sequence a request made in this step tracks up to: the queue's
    /// next sequence when a transfer to or from the unit's local store is
    /// outstanding at the step's start, and `None` when none is, so the
    /// request completes at once.
    pub mssync_horizon: Option<u64>,
    /// The pending-event register: events that occurred and are not
    /// acknowledged, masked or not.
    ///
    /// [CBEA p:146 s:9.11] an edge-triggered event sets its bit in the SPU Pending Event Register, and a write of 1 to that bit of SPU_WrEventAck resets it.
    pub pending_events: u32,
    /// The events `SPU_RdEventStat` reports.
    ///
    /// [CBEA p:146 s:9.11] a read of SPU_RdEventStat returns the pending-event register ANDed with the SPU_WrEventMask value.
    pub event_mask: u32,
    /// The `SPU_RdEventStat` count, which saturates at 1.
    ///
    /// [CBEA p:147 s:9.11.1] the count is 1 after an enabled event occurs, after a mask write enables a pending event, or after an acknowledgment leaves enabled events pending; a read sets it to 0.
    pub event_count: bool,
    /// The event sources whose channel count was nonzero when the unit
    /// last looked, one event bit each. A bit set now and clear here is
    /// that event's edge.
    ///
    /// [CBEA p:149 s:9.11.1] hardware determines events by detecting the channel counts that change from 0 to a nonzero value.
    pub event_levels: u32,
    /// The decrementer `SPU_WrDec` loads and `SPU_RdDec` reads.
    pub decrementer: crate::decrementer::Decrementer,
    /// The guest tick of the instruction the unit runs now. The unit
    /// sets it before each instruction, and the decrementer reads the
    /// time base from it.
    pub clock: cellgov_time::GuestTicks,
    /// Whether the step in flight read the guest clock: its result
    /// depends on the tick it ran at. The unit clears it at step entry
    /// and publishes one `ClockRead` effect for a step that set it.
    pub clock_read: bool,
}

/// Where a list command starts after its hold, or resumes after its stall.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListCursor {
    /// The command word that queued the list.
    pub word: u32,
    /// The list's tag group.
    pub tag: cellgov_ps3_abi::hw::spu::MfcTagId,
    /// Put or get.
    pub direction: cellgov_dma::DmaDirection,
    /// The fence or barrier the command's form sets.
    pub ordering: cellgov_dma::MfcOrdering,
    /// The high effective-address word every element shares.
    pub eah: u32,
    /// Local-store address of the next list element.
    pub element: u32,
    /// Elements not yet queued.
    pub remaining: u32,
    /// Local-store address the next element's transfer uses.
    pub data: u32,
    /// The stall-and-notify element completed, so the list is stalled
    /// and an acknowledgment resumes it.
    pub stalled: bool,
    /// An earlier command still orders the list after itself, so the list reads no element yet.
    ///
    /// On the console, a tag-barrier list reads its elements after an
    /// earlier get of its tag group lands them.
    /// [CBEA p:69 s:7.9] a fence or barrier orders the command after the preceding commands of its tag group.
    pub held: bool,
}

impl ChannelState {
    /// The channel state of a new context.
    ///
    /// Every channel count follows from these values; `rchcnt` on a
    /// fresh SPU reads the counts the architecture requires.
    ///
    /// [CBEA p:237 s:16.3.2] the data of channels x'0', x'1', x'3', x'4', x'18', x'19', x'1B' and x'1D' is zero before a new context starts.
    /// [CBEA p:238 s:16.3.3] the counts of x'0', x'3', x'4', x'18', x'19', x'1B' and x'1D' start at 0; x'17', x'1C' and x'1E' at 1; MFC_Cmd at the queue depth.
    pub fn new() -> Self {
        Self {
            mfc_lsa: 0,
            mfc_eah: 0,
            mfc_eal: 0,
            mfc_size: 0,
            mfc_tag_id: 0,
            tag_mask: 0,
            // x'18' data.
            tag_status: 0,
            // x'1B' data.
            atomic_status: 0,
            // MFC_Cmd count: the queue depth.
            cmd_queue_free: cellgov_ps3_abi::hw::spu::MFC_SPU_QUEUE_DEPTH,
            tag_update: None,
            // x'18' count 0.
            tag_status_read: None,
            // x'1B' count 0.
            atomic_status_ready: false,
            // x'1D' count 0.
            in_mbox: Vec::new(),
            // x'1C' count 1.
            out_mbox: None,
            out_intr_mbox_connected: false,
            // x'1E' count 1 once connected.
            out_intr_mbox: None,
            out_intr_mbox_interrupt: false,
            lists: Vec::new(),
            mfc_ordering: cellgov_exec::MfcOrderingView::default(),
            // x'19' count 0.
            list_stall_status: 0,
            // x'9' count 1.
            mssync_tracking: None,
            mssync_horizon: None,
            pending_events: 0,
            event_mask: 0,
            // x'0' count 0.
            event_count: false,
            // The sources whose counts start nonzero without an event:
            // MFC_Cmd, MFC_WrMSSyncReq and SPU_WrOutIntrMbox. On the
            // reference console, an SPU thread and a raw SPU start with Le
            // pending (`spu_mailbox_events`, `spu_raw_mailbox_events`).
            // Le's level starts clear, so the unit's first look raises Le.
            event_levels: cellgov_ps3_abi::hw::spu::event::QV
                | cellgov_ps3_abi::hw::spu::event::MS
                | cellgov_ps3_abi::hw::spu::event::ME,
            decrementer: crate::decrementer::Decrementer::new(),
            clock: cellgov_time::GuestTicks::ZERO,
            clock_read: false,
        }
    }
}

impl ChannelState {
    /// Replace the pending-event register and the mask. A bit of the
    /// event status, pending AND mask, that turns on sets the
    /// `SPU_RdEventStat` count.
    ///
    /// [CBEA p:146 s:9.11] any transition of a bit from 0 to 1 in SPU_RdEventStat increments its count, which saturates at 1.
    pub fn set_event_state(&mut self, pending: u32, mask: u32) {
        let before = self.pending_events & self.event_mask;
        self.pending_events = pending;
        self.event_mask = mask;
        if pending & mask & !before != 0 {
            self.event_count = true;
        }
    }

    /// Applies a write to `MFC_WrTagUpdate`.
    ///
    /// - An immediate request latches the masked status now.
    /// - A conditional request latches it once its condition holds.
    ///
    /// A new request replaces an earlier one and its unread result.
    /// The caller refuses a reserved value first. A reserved value that
    /// reaches this function makes no request.
    ///
    /// [CBE-Handbook p:459 s:17.10] TS 00 updates immediately, 01 when any enabled group completes, 10 when all do; 11 is reserved.
    pub fn request_tag_update(&mut self, value: u32) {
        let condition = match value & 3 {
            MFC_TAG_UPDATE_IMMEDIATE => None,
            MFC_TAG_UPDATE_ANY => Some(TagUpdateCondition::Any),
            MFC_TAG_UPDATE_ALL => Some(TagUpdateCondition::All),
            _ => return,
        };
        self.tag_status_read = None;
        match condition {
            None => {
                self.tag_update = None;
                self.tag_status_read = Some(self.tag_status & self.tag_mask);
            }
            Some(condition) => {
                self.tag_update = Some(condition);
                self.settle_tag_update();
            }
        }
    }

    /// Latches the masked status if the waiting request's condition holds.
    ///
    /// With an empty mask, "all enabled groups" holds at once and "any
    /// enabled group" never does.
    ///
    /// [CBEA p:128 s:9.3.6] a set bit means the group has no outstanding operations and the query mask enables it.
    pub fn settle_tag_update(&mut self) {
        let masked = self.tag_status & self.tag_mask;
        let met = match self.tag_update {
            None => return,
            Some(TagUpdateCondition::Any) => masked != 0,
            Some(TagUpdateCondition::All) => masked == self.tag_mask,
        };
        if met {
            self.tag_update = None;
            self.tag_status_read = Some(masked);
        }
    }
}

/// The condition a tag-status update request waits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TagUpdateCondition {
    /// Any group the query mask enables has no outstanding operations.
    Any,
    /// Every group the query mask enables has no outstanding operations.
    All,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self::new()
    }
}
