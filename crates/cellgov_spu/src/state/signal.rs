//! The signal-notification registers.

/// How a signal-notification register takes a write.
///
/// [CBEA p:239 s:16.4] each register either overwrites its contents or ORs the data written into them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalNotifyMode {
    /// A write replaces the contents.
    Overwrite,
    /// A write ORs its data into the contents.
    LogicalOr,
}

/// One signal-notification register: its `SPU_Cfg` mode, its
/// signal-control word, and whether a write is unread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalNotifyRegister {
    /// How a write changes `word`.
    ///
    /// [CBEA p:239 s:16.4] SPU_Cfg sets each signal-notification register to overwrite or logical OR.
    pub mode: SignalNotifyMode,
    /// The signal-control word the channel reads.
    pub word: u32,
    /// A write is unread, so the channel counts 1.
    pub pending: bool,
}

impl SignalNotifyRegister {
    /// A register at the power-on reset state.
    ///
    /// [CBEA p:239 s:16.4] each register starts in overwrite mode, the power-on reset value.
    /// [CBEA p:237 s:16.3.2], [CBEA p:238 s:16.3.3] channels x'3' and x'4' start with data 0 and count 0.
    pub const fn new() -> Self {
        Self {
            mode: SignalNotifyMode::Overwrite,
            word: 0,
            pending: false,
        }
    }

    /// Take a write from another processor.
    ///
    /// [CBEA p:101 s:8.7] overwrite mode sets the channel to the data and logical OR mode ORs the data in; both set the count to 1.
    pub fn write(&mut self, value: u32) {
        self.word = match self.mode {
            SignalNotifyMode::Overwrite => value,
            SignalNotifyMode::LogicalOr => self.word | value,
        };
        self.pending = true;
    }
}

impl Default for SignalNotifyRegister {
    fn default() -> Self {
        Self::new()
    }
}
