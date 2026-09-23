//! SPU local store and register-file sizes, channel numbers and MFC
//! command opcodes.
//!
//! Channel-access semantics live in `cellgov_spu`; this module holds
//! the ABI facts and the pure functions over them.
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
/// Highest tag id an MFC command may name; the field is bits 27:31.
// [CBEA p:115 s:9.1.3 MFC Command Tag Identification Channel] the identification tag is any value between x'0' and x'1F'.
pub const MFC_MAX_TAG_ID: u32 = 31;

/// MFC command opcode register; writing submits the DMA command.
// [CBEA p:113 s:9.1.1 MFC Command Opcode Channel] channel x'15' = 21; write triggers issue.
pub const MFC_CMD: u8 = 21;

/// A tag id inside the architected range.
///
/// The completion path publishes `1 << tag_id` into a 32-bit tag-status
/// word, so a value this type refuses has no bit to set. Holding the
/// bound here means [`MfcTagId::status_bit`] cannot overflow, whoever
/// built the command.
// [CBEA p:128 s:9.3.6 MFC Read Tag-Group Status Channel] the status word reports one bit per tag group, and a group left out of the query mask reads zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MfcTagId(u8);

impl MfcTagId {
    /// Construct from a raw tag id.
    ///
    /// # Errors
    /// `None` above [`MFC_MAX_TAG_ID`].
    #[inline]
    pub const fn new(raw: u8) -> Option<Self> {
        if raw as u32 > MFC_MAX_TAG_ID {
            return None;
        }
        Some(Self(raw))
    }

    /// The tag-status bit this id publishes.
    #[inline]
    pub const fn status_bit(self) -> u32 {
        1u32 << self.0
    }
}

// MFC tag status channels

/// Write tag query mask.
// [CBEA p:122 s:9.3 MFC Tag-Group Status Channels] MFC_WrTagMask, channel 22.
pub const MFC_WR_TAG_MASK: u8 = 22;
/// Write tag status update request (0=immediate, 1=any, 2=all).
// [CBEA p:122 s:9.3 MFC Tag-Group Status Channels] MFC_WrTagUpdate, channel 23.
pub const MFC_WR_TAG_UPDATE: u8 = 23;
/// Requests a tag status update without waiting.
// [CBE-Handbook p:459 s:17.10 MFC Tag-Group Management Channels] TS=00 requests an immediate update.
pub const MFC_TAG_UPDATE_IMMEDIATE: u32 = 0;
/// Requests a tag status update after any enabled group completes.
// [CBE-Handbook p:459 s:17.10 MFC Tag-Group Management Channels] TS=01 waits for any enabled group.
pub const MFC_TAG_UPDATE_ANY: u32 = 1;
/// Requests a tag status update after all enabled groups complete.
// [CBE-Handbook p:459 s:17.10 MFC Tag-Group Management Channels] TS=10 waits for all enabled groups.
pub const MFC_TAG_UPDATE_ALL: u32 = 2;
/// Read tag status; blocks until masked tags complete.
// [CBEA p:122 s:9.3 MFC Tag-Group Status Channels] MFC_RdTagStat, channel 24, read-blocking.
pub const MFC_RD_TAG_STAT: u8 = 24;

// MFC atomic channels

/// Read atomic operation status (after getllar/putllc).
// [CBEA p:131 s:9.4 MFC Read Atomic Command Status Channel] MFC_RdAtomicStat, channel 27.
pub const MFC_RD_ATOMIC_STAT: u8 = 27;
/// `MFC_RdAtomicStat` G bit: a `getllar` completed.
// [CBEA p:131 s:9.4 MFC Read Atomic Command Status Channel] bit 29 of the 32-bit status word is G, set when the get lock-line and reserve command completed.
pub const MFC_ATOMIC_STAT_G: u32 = 1 << (31 - 29);
/// `MFC_RdAtomicStat` S bit: a `putllc` lost its reservation. The bit
/// is clear when the conditional store succeeded.
// [CBEA p:131 s:9.4 MFC Read Atomic Command Status Channel] bit 31 of the status word is S, 1 when the put conditional was unsuccessful and 0 when it succeeded.
pub const MFC_ATOMIC_STAT_S: u32 = 1;

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

// SPU state management channels

/// SPU read machine status: isolation status and interrupt enable.
// [CBEA p:141 s:9.8 SPU Read Machine Status Channel] SPU_RdMachStat, channel x'D' = 13, nonblocking.
pub const SPU_RD_MACH_STAT: u8 = 13;

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

/// One word written to [`MFC_CMD`]: an opcode and two class ids.
///
/// | bits | field |
/// | --- | --- |
/// | 0:7 | TclassID |
/// | 8:15 | RclassID |
/// | 16:23 | reserved, bit 16 marking the opcode reserved |
/// | 24:31 | opcode |
///
/// Bit numbering is the document's, most significant first, so the
/// opcode is the word's low byte and bit 16 is `1 << 15`.
// [CBE-Handbook p:457 s:17.9.6 MFC Class ID and MFC Command Opcode Channel] the write sets the class ids and the opcode and enqueues the command formed by the earlier parameter writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MfcCmd(u32);

impl MfcCmd {
    /// Wrap a word the guest wrote to the channel.
    #[inline]
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// The word as the guest wrote it.
    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }

    /// The operation this command names, against [`MFC_PUT`] and its
    /// siblings.
    #[inline]
    pub const fn opcode(self) -> u32 {
        self.0 & 0xFF
    }

    /// Transfer class id, which steers bus bandwidth.
    // [CBE-Handbook p:457 s:17.9.6 MFC Class ID and MFC Command Opcode Channel] TclassID steers how large a share of the bus a transfer is given.
    #[inline]
    pub const fn tclass_id(self) -> u8 {
        (self.0 >> 24) as u8
    }

    /// Replacement class id, which steers L2-cache and TLB replacement.
    // [CBE-Handbook p:457 s:17.9.6 MFC Class ID and MFC Command Opcode Channel] RclassID steers which L2-cache and address-translation entries are chosen for replacement.
    #[inline]
    pub const fn rclass_id(self) -> u8 {
        (self.0 >> 16) as u8
    }

    /// True where the word marks its own opcode reserved, whatever the
    /// opcode byte holds.
    // [CBEA p:113 s:9.1.1 MFC Command Opcode Channel] the command parameter is the word's low halfword, whose own leading bit marks the opcode reserved.
    #[inline]
    pub const fn names_a_reserved_opcode(self) -> bool {
        self.0 & (1 << 15) != 0
    }
}

/// SPU local store size in bytes (256 KiB).
// [CBE-Handbook p:64 s:3.1.1] Local Store is a 256 KB single-ported memory.
pub const SPU_LS_SIZE: usize = 256 * 1024;

/// Number of SPU general-purpose 128-bit registers (r0..r127).
// [SPU-ISA p:25 s:2] The SPU architecture defines 128 general-purpose
// registers, each holding 128 data bits.
pub const SPU_REG_COUNT: usize = 128;
