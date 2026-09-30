//! A unit parked on a blocking channel, and the event that ends the park.

/// The event that gives a stalled channel a non-zero count.
///
/// The runtime wakes a stalled unit only on its channel's own event, and
/// the unit then runs the stalled access again. Later producers join as
/// the model gains them: a signal-notification write, an SPU event, a
/// free MFC command-queue slot, and an interrupt, which ends any stall.
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
