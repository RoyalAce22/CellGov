//! SPU channel numbers and MFC command opcodes.
//!
//! Architectural constants used by `rdch`/`wrch`/`rchcnt`.
// [CBEA p:112 s:9.1 MFC SPU Command Parameter Channels] SPU channel architecture overview.

// MFC command channels

/// MFC local store address register.
// [CBEA p:117 s:9.1.5 MFC Local Storage Address Channel] channel x'10' = 16.
pub const MFC_LSA: u8 = 16;
/// MFC effective address high word register.
// [CBEA p:120 s:9.1.7 MFC Effective Address High Channel] channel x'11' = 17.
pub const MFC_EAH: u8 = 17;
/// MFC effective address low word register.
// [CBEA p:118 s:9.1.6 MFC Effective Address Low or List Address Channel] channel x'12' = 18.
pub const MFC_EAL: u8 = 18;
/// MFC transfer size register.
// [CBEA p:116 s:9.1.4 MFC Transfer Size or List Size Channel] channel x'13' = 19.
pub const MFC_SIZE: u8 = 19;
/// MFC tag ID register.
// [CBEA p:115 s:9.1.3 MFC Command Tag Identification Channel] channel x'14' = 20.
pub const MFC_TAG_ID: u8 = 20;
/// MFC command opcode register; writing submits the DMA command.
// [CBEA p:113 s:9.1.1 MFC Command Opcode Channel] channel x'15' = 21; write triggers issue.
pub const MFC_CMD: u8 = 21;

// MFC tag status channels

/// Write tag query mask.
// [CBEA p:122 s:9.3 MFC Tag-Group Status Channels] MFC_WrTagMask, channel 22.
pub const MFC_WR_TAG_MASK: u8 = 22;
/// Write tag status update request (0=immediate, 1=any, 2=all).
// [CBEA p:122 s:9.3 MFC Tag-Group Status Channels] MFC_WrTagUpdate, channel 23.
pub const MFC_WR_TAG_UPDATE: u8 = 23;
/// Read tag status; blocks until masked tags complete.
// [CBEA p:122 s:9.3 MFC Tag-Group Status Channels] MFC_RdTagStat, channel 24, read-blocking.
pub const MFC_RD_TAG_STAT: u8 = 24;

// MFC atomic channels

/// Read atomic operation status (after getllar/putllc).
// [CBEA p:131 s:9.4 MFC Read Atomic Command Status Channel] MFC_RdAtomicStat, channel 27.
pub const MFC_RD_ATOMIC_STAT: u8 = 27;

// SPU mailbox channels

/// SPU read inbound mailbox (PPU -> SPU); blocks if empty.
// [CBEA p:135 s:9.5 SPU Mailbox Channels] SPU_RdInMbox, channel 29, read-blocking.
pub const SPU_RD_IN_MBOX: u8 = 29;
/// SPU write outbound mailbox (SPU -> PPU).
// [CBEA p:133 s:9.5 SPU Mailbox Channels] SPU_WrOutMbox, channel 28, write-blocking.
pub const SPU_WR_OUT_MBOX: u8 = 28;
/// SPU write outbound interrupt mailbox.
// [CBEA p:134 s:9.5 SPU Mailbox Channels] SPU_WrOutIntrMbox, channel 30.
pub const SPU_WR_OUT_INTR_MBOX: u8 = 30;

// MFC DMA command opcodes (written to MFC_CMD)

/// DMA put: local store -> main memory.
// [CBEA p:61 s:7.6 Put Commands] put opcode 0x20, LS to main storage.
pub const MFC_PUT: u32 = 0x20;
/// DMA get: main memory -> local store.
// [CBEA p:60 s:7.5 Get Commands] get opcode 0x40, main storage to LS.
pub const MFC_GET: u32 = 0x40;
/// Atomic: get with reservation (getllar).
// [CBEA p:65 s:7.8 MFC Atomic Update Commands] getllar opcode 0xD0.
pub const MFC_GETLLAR: u32 = 0xD0;
/// Atomic: put conditional (putllc).
// [CBEA p:65 s:7.8 MFC Atomic Update Commands] putllc opcode 0xB4.
pub const MFC_PUTLLC: u32 = 0xB4;
