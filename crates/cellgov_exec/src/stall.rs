//! A unit parked on a blocking channel, and the event that ends the park.

/// The event that gives a stalled channel a non-zero count.
///
/// The runtime wakes a stalled unit only on its channel's own event, and
/// the unit then runs the stalled access again. Later producers join as
/// the model gains them: an SPU event and an interrupt, which ends any
/// stall.
///
/// [CBE-Handbook p:447 s:17.1.6] a blocked access stalls until the channel changes or the SPU is interrupted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StallWake {
    /// A message arrives in the unit's inbound mailbox.
    MailboxDelivery,
    /// One of the unit's DMA transfers completes.
    DmaCompletion,
    /// Another unit reads the message waiting in the outbound mailbox.
    OutboundMailboxRead,
    /// One of the unit's queued MFC commands completes, freeing a slot
    /// in its command queue.
    ///
    /// [CBEA p:113 s:9.1.1] a write to MFC_Cmd with the command queue full stalls until a slot frees.
    CommandQueueSlot,
    /// Another processor writes the signal-notification register the
    /// unit reads.
    SignalWrite(crate::SignalNotifier),
    /// An immediate atomic command of the unit completes.
    ///
    /// The unit issues those commands itself, so while it stalls none
    /// completes: only an interrupt, which the model does not raise, ends
    /// the stall.
    ///
    /// [CBEA p:131 s:9.4] a read of MFC_RdAtomicStat before the unit issues an immediate atomic command is a software-induced deadlock.
    AtomicCommandCompletion,
}

impl StallWake {
    /// Whether a completion of one of the unit's DMA transfers ends the
    /// stall.
    pub const fn ends_on_dma_completion(self) -> bool {
        matches!(self, Self::DmaCompletion | Self::CommandQueueSlot)
    }
}

/// The blocking channel a unit stalled on and the event that wakes it.
///
/// [CBEA p:109 s:9] a read-blocking or write-blocking channel access completes only when the channel count is non-zero; otherwise the SPU stalls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChannelStall {
    /// The channel number the stalled instruction names.
    pub channel: u8,
    /// The event that makes the channel's count non-zero.
    pub wake: StallWake,
}
