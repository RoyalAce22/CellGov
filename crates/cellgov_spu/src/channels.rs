//! SPU channel numbers and MFC command opcodes.
//!
//! Architectural constants used by `rdch`/`wrch`/`rchcnt`.

// MFC command channels

/// MFC local store address register.
pub const MFC_LSA: u8 = 16;
/// MFC effective address high word register.
pub const MFC_EAH: u8 = 17;
/// MFC effective address low word register.
pub const MFC_EAL: u8 = 18;
/// MFC transfer size register.
pub const MFC_SIZE: u8 = 19;
/// MFC tag ID register.
pub const MFC_TAG_ID: u8 = 20;
/// MFC command opcode register; writing submits the DMA command.
pub const MFC_CMD: u8 = 21;

// MFC tag status channels

/// Write tag query mask.
pub const MFC_WR_TAG_MASK: u8 = 22;
/// Write tag status update request (0=immediate, 1=any, 2=all).
pub const MFC_WR_TAG_UPDATE: u8 = 23;
/// Read tag status; blocks until masked tags complete.
pub const MFC_RD_TAG_STAT: u8 = 24;

// MFC atomic channels

/// Read atomic operation status (after getllar/putllc).
pub const MFC_RD_ATOMIC_STAT: u8 = 27;

// SPU mailbox channels

/// SPU read inbound mailbox (PPU -> SPU); blocks if empty.
pub const SPU_RD_IN_MBOX: u8 = 29;
/// SPU write outbound mailbox (SPU -> PPU).
pub const SPU_WR_OUT_MBOX: u8 = 28;
/// SPU write outbound interrupt mailbox.
pub const SPU_WR_OUT_INTR_MBOX: u8 = 30;

// MFC DMA command opcodes (written to MFC_CMD)

/// DMA put: local store -> main memory.
pub const MFC_PUT: u32 = 0x20;
/// DMA get: main memory -> local store.
pub const MFC_GET: u32 = 0x40;
/// Atomic: get with reservation (getllar).
pub const MFC_GETLLAR: u32 = 0xD0;
/// Atomic: put conditional (putllc).
pub const MFC_PUTLLC: u32 = 0xB4;
