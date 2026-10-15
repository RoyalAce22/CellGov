//! The SPE problem-state operations another processor performs on a
//! unit, and the refusals they can meet.

/// One of the two SPU signal-notification registers.
///
/// [CBEA p:101 s:8.7] each SPU has two independent signal-notification facilities, each one register and one channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SignalNotifier {
    /// `SPU_Sig_Notify_1`, read by channel x'3'.
    One,
    /// `SPU_Sig_Notify_2`, read by channel x'4'.
    Two,
}

/// The unread messages in an SPU's outbound mailboxes.
///
/// [CBEA p:100 s:8.6.3] SPU_Mbox_Stat counts the valid entries in each outbound mailbox queue, and reading it changes no queue.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct OutMailboxes {
    /// The `SPU_Out_Mbox` message.
    pub out_mbox: Option<u32>,
    /// The `SPU_Out_Intr_Mbox` message.
    pub out_intr_mbox: Option<u32>,
}

/// Why a unit refused a problem-state operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ProblemStateError {
    /// The unit has no SPE problem-state registers.
    #[error("unit has no SPE problem state")]
    NoProblemState,
    /// The runtime holds no unit with the id.
    #[error("no unit with that id")]
    UnknownUnit,
    /// A `Finished` status override holds the unit, as a process exit
    /// sets for every unit of the process.
    #[error("unit is retired by a status override")]
    Retired,
    /// The operation needs a stopped SPU and the SPU is running.
    #[error("SPU is running")]
    Running,
    /// CellGov refused the unit, so it neither runs nor holds an
    /// architected stopped state.
    #[error("unit is refused by CellGov")]
    Refused,
}
