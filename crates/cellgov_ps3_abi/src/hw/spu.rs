//! SPU local store and register-file sizes, channel numbers and MFC
//! command opcodes.
//!
//! Channel-access semantics live in `cellgov_spu`; this module holds
//! the ABI facts and the pure functions over them.
// [CBEA p:112 s:9.1 MFC SPU Command Parameter Channels] SPU channel architecture overview.

// MFC command channels

/// MFC local store address register.
///
/// [CBEA p:117 s:9.1.5 MFC Local Storage Address Channel] channel x'10' = 16.
pub const MFC_LSA: u8 = 16;
/// MFC effective address high word register.
///
/// [CBEA p:120 s:9.1.7 MFC Effective Address High Channel] channel x'11' = 17.
pub const MFC_EAH: u8 = 17;
/// MFC effective address low word register.
///
/// [CBEA p:118 s:9.1.6 MFC Effective Address Low or List Address Channel] channel x'12' = 18.
pub const MFC_EAL: u8 = 18;
/// MFC transfer size register.
///
/// [CBEA p:116 s:9.1.4 MFC Transfer Size or List Size Channel] channel x'13' = 19.
pub const MFC_SIZE: u8 = 19;
/// MFC tag ID register.
///
/// [CBEA p:115 s:9.1.3 MFC Command Tag Identification Channel] channel x'14' = 20.
pub const MFC_TAG_ID: u8 = 20;
/// Highest tag id an MFC command may name; the field is bits 27:31.
///
/// [CBEA p:115 s:9.1.3 MFC Command Tag Identification Channel] the identification tag is any value between x'0' and x'1F'.
pub const MFC_MAX_TAG_ID: u32 = 31;

/// MFC command opcode register; writing submits the DMA command.
///
/// [CBEA p:113 s:9.1.1 MFC Command Opcode Channel] channel x'15' = 21; write triggers issue.
pub const MFC_CMD: u8 = 21;

/// A tag id inside the architected range.
///
/// The tag-status word holds one bit per tag group, so a value this type
/// refuses names no group. Holding the bound here means
/// [`MfcTagId::status_bit`] cannot overflow, whoever built the command.
///
/// [CBEA p:128 s:9.3.6 MFC Read Tag-Group Status Channel] the status word reports one bit per tag group, and a group left out of the query mask reads zero.
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

    /// The tag id, 0 to 31.
    #[inline]
    pub const fn raw(self) -> u8 {
        self.0
    }

    /// The bit of this id's group in the tag-status word.
    #[inline]
    pub const fn status_bit(self) -> u32 {
        1u32 << self.0
    }
}

// MFC tag status channels

/// Write tag query mask.
///
/// [CBEA p:122 s:9.3 MFC Tag-Group Status Channels] MFC_WrTagMask, channel 22.
pub const MFC_WR_TAG_MASK: u8 = 22;
/// Write tag status update request (0=immediate, 1=any, 2=all).
///
/// [CBEA p:122 s:9.3 MFC Tag-Group Status Channels] MFC_WrTagUpdate, channel 23.
pub const MFC_WR_TAG_UPDATE: u8 = 23;
/// Requests a tag status update without waiting.
///
/// [CBE-Handbook p:459 s:17.10 MFC Tag-Group Management Channels] TS=00 requests an immediate update.
pub const MFC_TAG_UPDATE_IMMEDIATE: u32 = 0;
/// Requests a tag status update after any enabled group completes.
///
/// [CBE-Handbook p:459 s:17.10 MFC Tag-Group Management Channels] TS=01 waits for any enabled group.
pub const MFC_TAG_UPDATE_ANY: u32 = 1;
/// Requests a tag status update after all enabled groups complete.
///
/// [CBE-Handbook p:459 s:17.10 MFC Tag-Group Management Channels] TS=10 waits for all enabled groups.
pub const MFC_TAG_UPDATE_ALL: u32 = 2;
/// Read tag status; blocks until masked tags complete.
///
/// [CBEA p:122 s:9.3 MFC Tag-Group Status Channels] MFC_RdTagStat, channel 24, read-blocking.
pub const MFC_RD_TAG_STAT: u8 = 24;
/// Read list stall-and-notify tag status.
///
/// [CBEA p:129 s:9.3.7 MFC Read List Stall-and-Notify Tag Status Channel] MFC_RdListStallStat, channel 25, read-blocking.
pub const MFC_RD_LIST_STALL_STAT: u8 = 25;

// MFC atomic channels

/// Read atomic operation status (after getllar/putllc).
///
/// [CBEA p:131 s:9.4 MFC Read Atomic Command Status Channel] MFC_RdAtomicStat, channel 27.
pub const MFC_RD_ATOMIC_STAT: u8 = 27;
/// `MFC_RdAtomicStat` G bit: a `getllar` completed.
///
/// [CBEA p:131 s:9.4 MFC Read Atomic Command Status Channel] bit 29 of the 32-bit status word is G, set when the get lock-line and reserve command completed.
pub const MFC_ATOMIC_STAT_G: u32 = 1 << (31 - 29);
/// `MFC_RdAtomicStat` S bit: a `putllc` lost its reservation. The bit
/// is clear when the conditional store succeeded.
///
/// [CBEA p:131 s:9.4 MFC Read Atomic Command Status Channel] bit 31 of the status word is S, 1 when the put conditional was unsuccessful and 0 when it succeeded.
pub const MFC_ATOMIC_STAT_S: u32 = 1;

// SPU mailbox channels

/// SPU read inbound mailbox (PPU -> SPU); blocks if empty.
///
/// [CBEA p:135 s:9.5 SPU Mailbox Channels] SPU_RdInMbox, channel 29, read-blocking.
pub const SPU_RD_IN_MBOX: u8 = 29;
/// SPU write outbound mailbox (SPU -> PPU).
///
/// [CBEA p:133 s:9.5 SPU Mailbox Channels] SPU_WrOutMbox, channel 28, write-blocking.
pub const SPU_WR_OUT_MBOX: u8 = 28;
/// SPU write outbound interrupt mailbox.
///
/// [CBEA p:134 s:9.5 SPU Mailbox Channels] SPU_WrOutIntrMbox, channel 30.
pub const SPU_WR_OUT_INTR_MBOX: u8 = 30;

// SPU signal notification channels

/// SPU signal notification 1.
///
/// [CBEA p:137 s:9.6.1 SPU Signal Notification 1 Channel] SPU_RdSigNotify1, channel x'3', read-blocking.
pub const SPU_RD_SIG_NOTIFY_1: u8 = 3;
/// SPU signal notification 2.
///
/// [CBEA p:138 s:9.6.2 SPU Signal Notification 2 Channel] SPU_RdSigNotify2, channel x'4', read-blocking.
pub const SPU_RD_SIG_NOTIFY_2: u8 = 4;

// SPU event channels

/// SPU read event status: the pending events the event mask enables.
///
/// [CBEA p:147 s:9.11.1 SPU Read Event Status Channel] SPU_RdEventStat, channel x'0', read-blocking.
pub const SPU_RD_EVENT_STAT: u8 = 0;

// SPU state management channels

/// SPU read machine status: isolation status and interrupt enable.
///
/// [CBEA p:141 s:9.8 SPU Read Machine Status Channel] SPU_RdMachStat, channel x'D' = 13, nonblocking.
pub const SPU_RD_MACH_STAT: u8 = 13;

// The other architected channels

/// SPU write event mask.
///
/// [CBEA p:299 s:Appendix B, Table B-1] SPU_WrEventMask, channel x'1', write.
pub const SPU_WR_EVENT_MASK: u8 = 1;
/// SPU write event acknowledgment.
///
/// [CBEA p:299 s:Appendix B, Table B-1] SPU_WrEventAck, channel x'2', write.
pub const SPU_WR_EVENT_ACK: u8 = 2;
/// SPU write decrementer.
///
/// [CBEA p:299 s:Appendix B, Table B-1] SPU_WrDec, channel x'7', write.
pub const SPU_WR_DEC: u8 = 7;
/// SPU read decrementer.
///
/// [CBEA p:299 s:Appendix B, Table B-1] SPU_RdDec, channel x'8', read.
pub const SPU_RD_DEC: u8 = 8;
/// MFC write multisource synchronization request.
///
/// [CBEA p:299 s:Appendix B, Table B-1] MFC_WrMSSyncReq, channel x'9', write-blocking.
pub const MFC_WR_MSSYNC_REQ: u8 = 9;
/// SPU read event mask.
///
/// [CBEA p:299 s:Appendix B, Table B-1] SPU_RdEventMask, channel x'B', read.
pub const SPU_RD_EVENT_MASK: u8 = 11;
/// MFC read tag-group query mask.
///
/// [CBEA p:300 s:Appendix B, Table B-1] MFC_RdTagMask, channel x'C', read.
pub const MFC_RD_TAG_MASK: u8 = 12;
/// SPU write state save-and-restore.
///
/// [CBEA p:300 s:Appendix B, Table B-1] SPU_WrSRR0, channel x'E', write.
pub const SPU_WR_SRR0: u8 = 14;
/// SPU read state save-and-restore.
///
/// [CBEA p:300 s:Appendix B, Table B-1] SPU_RdSRR0, channel x'F', read.
pub const SPU_RD_SRR0: u8 = 15;
/// MFC write list stall-and-notify tag acknowledgment.
///
/// [CBEA p:300 s:Appendix B, Table B-1] MFC_WrListStallAck, channel x'1A', write.
pub const MFC_WR_LIST_STALL_ACK: u8 = 26;

/// Whether a channel takes rdch or wrch.
///
/// [CBEA p:109 s:9] each channel is read-only or write-only, never both.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChannelDirection {
    /// A read or read-blocking channel.
    Read,
    /// A write or write-blocking channel.
    Write,
}

/// The direction of an implemented channel; `None` for a reserved one.
///
/// [CBEA p:299 s:Appendix B, Table B-1] the access type of channels x'0' to x'B'.
/// [CBEA p:300 s:Appendix B, Table B-1] the access type of channels x'C' to x'1C'.
/// [CBEA p:301 s:Appendix B, Table B-1] the access type of channels x'1D' and x'1E'.
pub const fn channel_direction(channel: u8) -> Option<ChannelDirection> {
    match channel {
        SPU_RD_EVENT_STAT
        | SPU_RD_SIG_NOTIFY_1
        | SPU_RD_SIG_NOTIFY_2
        | SPU_RD_DEC
        | SPU_RD_EVENT_MASK
        | MFC_RD_TAG_MASK
        | SPU_RD_MACH_STAT
        | SPU_RD_SRR0
        | MFC_RD_TAG_STAT
        | MFC_RD_LIST_STALL_STAT
        | MFC_RD_ATOMIC_STAT
        | SPU_RD_IN_MBOX => Some(ChannelDirection::Read),
        SPU_WR_EVENT_MASK
        | SPU_WR_EVENT_ACK
        | SPU_WR_DEC
        | MFC_WR_MSSYNC_REQ
        | SPU_WR_SRR0
        | MFC_LSA
        | MFC_EAH
        | MFC_EAL
        | MFC_SIZE
        | MFC_TAG_ID
        | MFC_CMD
        | MFC_WR_TAG_MASK
        | MFC_WR_TAG_UPDATE
        | MFC_WR_LIST_STALL_ACK
        | SPU_WR_OUT_MBOX
        | SPU_WR_OUT_INTR_MBOX => Some(ChannelDirection::Write),
        _ => None,
    }
}

// Reserved channels

/// True for a channel number the CBE leaves reserved: 5, 6, 10, and 31
/// upward to the top of the 7-bit channel field.
///
/// [CBEA p:299 s:Appendix B, Table B-1] channels 5, 6 and 10 are reserved.
/// [CBEA p:301 s:Appendix B, Table B-1] channels 31 to 63 are reserved.
/// [CBE-Handbook p:446 s:17.1.4, Table 17-2] on the CBE, channels 31 to 127 are reserved.
pub const fn is_reserved_channel(channel: u8) -> bool {
    // [CBE-Handbook p:446 s:17.1.4, Table 17-2] the 7-bit channel field names 128 channels, so 128 and up name none.
    matches!(channel, 5 | 6 | 10 | 31..=127)
}

// MFC DMA command opcodes (written to MFC_CMD)

/// DMA put: local store -> main memory.
///
/// [CBEA p:61 s:7.6 Put Commands] put opcode 0x20, LS to main storage.
pub const MFC_PUT: u32 = 0x20;
/// DMA get: main memory -> local store.
///
/// [CBEA p:60 s:7.5 Get Commands] get opcode 0x40, main storage to LS.
pub const MFC_GET: u32 = 0x40;
/// DMA put with a tag-specific barrier.
///
/// [CBEA p:306 s:Appendix D Table D-2] putb opcode x'0021', supported on the proxy queue and the SPU queue.
pub const MFC_PUTB: u32 = 0x21;
/// DMA put with a tag-specific fence.
///
/// [CBEA p:306 s:Appendix D Table D-2] putf opcode x'0022'.
pub const MFC_PUTF: u32 = 0x22;
/// DMA get with a tag-specific barrier.
///
/// [CBEA p:307 s:Appendix D Table D-2] getb opcode x'0041'.
pub const MFC_GETB: u32 = 0x41;
/// DMA get with a tag-specific fence.
///
/// [CBEA p:307 s:Appendix D Table D-2] getf opcode x'0042'.
pub const MFC_GETF: u32 = 0x42;
/// DMA put with the replace-cache hint.
///
/// [CBEA p:54 s:7.1 Table 7-2] putr x'0030', putrb x'0031', putrf x'0032'.
pub const MFC_PUTR: u32 = 0x30;
/// DMA put with the replace-cache hint and a tag-specific barrier.
pub const MFC_PUTRB: u32 = 0x31;
/// DMA put with the replace-cache hint and a tag-specific fence.
pub const MFC_PUTRF: u32 = 0x32;
/// DMA list put: each list element names one transfer from local store
/// to main storage.
///
/// [CBEA p:306 s:Appendix D Table D-2] putl x'0024', putlb x'0025', putlf x'0026'; putrl x'0034', putrlb x'0035', putrlf x'0036'.
pub const MFC_PUTL: u32 = 0x24;
/// DMA list put with a tag-specific barrier.
pub const MFC_PUTLB: u32 = 0x25;
/// DMA list put with a tag-specific fence.
pub const MFC_PUTLF: u32 = 0x26;
/// DMA list put with the replace-cache hint.
///
/// [CBEA p:62 s:7.6.5] on the CBE, putrl, putrlf and putrlb behave as putl, putlf and putlb.
pub const MFC_PUTRL: u32 = 0x34;
/// DMA list put with the replace-cache hint and a tag-specific barrier.
pub const MFC_PUTRLB: u32 = 0x35;
/// DMA list put with the replace-cache hint and a tag-specific fence.
pub const MFC_PUTRLF: u32 = 0x36;
/// DMA list get: each list element names one transfer from main
/// storage to local store.
///
/// [CBEA p:307 s:Appendix D Table D-2] getl x'0044', getlb x'0045', getlf x'0046'.
pub const MFC_GETL: u32 = 0x44;
/// DMA list get with a tag-specific barrier.
pub const MFC_GETLB: u32 = 0x45;
/// DMA list get with a tag-specific fence.
pub const MFC_GETLF: u32 = 0x46;
/// Bytes in one DMA list element.
///
/// [CBEA p:59 s:7.4] a list element is a doubleword: the stall-and-notify flag and transfer size, then the low effective-address word.
pub const MFC_LIST_ELEMENT_BYTES: u32 = 8;
/// The stall-and-notify flag in a list element's first word, bit 0.
///
/// [CBEA p:59 s:7.4] bit 0 of a list element is the stall-and-notify flag, bits 1:16 are reserved, and bits 17:31 are the transfer size.
pub const MFC_LIST_STALL_NOTIFY: u32 = 0x8000_0000;
/// Send signal: a 4-byte put.
///
/// [CBEA p:308 s:Appendix D Table D-4] sndsig x'00A0', sndsigb x'00A1', sndsigf x'00A2'.
pub const MFC_SNDSIG: u32 = 0xA0;
/// Send signal with a tag-specific barrier.
pub const MFC_SNDSIGB: u32 = 0xA1;
/// Send signal with a tag-specific fence.
pub const MFC_SNDSIGF: u32 = 0xA2;
/// The barrier command.
///
/// [CBEA p:308 s:Appendix D Table D-4] barrier x'00C0', mfceieio x'00C8', mfcsync x'00CC'.
pub const MFC_BARRIER: u32 = 0xC0;
/// The mfceieio command.
pub const MFC_EIEIO: u32 = 0xC8;
/// The mfcsync command.
pub const MFC_SYNC: u32 = 0xCC;
/// SL1 storage control: touch a range of effective addresses, a hint
/// for a later get.
///
/// [CBEA p:307 s:Appendix D Table D-3] sdcrt x'0080', sdcrtst x'0081', sdcrz x'0089', sdcrst x'008D', sdcrf x'008F', each supported on the proxy queue and the SPU queue.
pub const MFC_SDCRT: u32 = 0x80;
/// SL1 storage control: touch a range for store, a hint for a later put.
pub const MFC_SDCRTST: u32 = 0x81;
/// SL1 storage control: write zeros over a range of effective addresses.
pub const MFC_SDCRZ: u32 = 0x89;
/// SL1 storage control: write the modified blocks of a range to main
/// storage.
pub const MFC_SDCRST: u32 = 0x8D;
/// SL1 storage control: write the modified blocks of a range to main
/// storage and invalidate them.
pub const MFC_SDCRF: u32 = 0x8F;
/// Bytes in the data block an SL1 storage control command acts on.
///
/// The CBE has no SL1. The commands that are not nops act on the SPE's
/// atomic cache, whose lines are 128 bytes.
///
/// [CBE-Handbook p:151 s:6.2.2.4 Table 6-1] the CBE does not implement an SL1; sdcrt and sdcrtst are nops, and sdcrz, sdcrst and sdcrf act on the atomic cache.
/// [CBE-Handbook p:149 s:6.2.2] the atomic cache stores six 128-byte cache lines.
pub const MFC_SL1_DATA_BLOCK_BYTES: u64 = 128;
/// Atomic: get with reservation (getllar).
///
/// [CBEA p:65 s:7.8 MFC Atomic Update Commands] getllar opcode 0xD0.
pub const MFC_GETLLAR: u32 = 0xD0;
/// Atomic: put conditional (putllc).
///
/// [CBEA p:65 s:7.8 MFC Atomic Update Commands] putllc opcode 0xB4.
pub const MFC_PUTLLC: u32 = 0xB4;
/// Atomic: put unconditional (putlluc).
///
/// [CBEA p:308 s:Appendix D Table D-5] putlluc opcode x'00B0'.
pub const MFC_PUTLLUC: u32 = 0xB0;

/// One word written to [`MFC_CMD`]: an opcode and two class ids.
///
/// | bits | field |
/// | --- | --- |
/// | 0:7 | TclassID |
/// | 8:15 | RclassID |
/// | 16:31 | opcode, bit 16 set for a reserved one |
///
/// Bit numbering is the document's, most significant first, so the
/// opcode is the word's low halfword and bit 16 is `1 << 15`.
///
/// [CBE-Handbook p:457 s:17.9.6 MFC Class ID and MFC Command Opcode Channel] the write sets the class ids and the opcode and enqueues the command formed by the earlier parameter writes.
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

    /// The 16-bit opcode, to compare against [`MFC_PUT`] and its siblings.
    ///
    /// [CBEA p:113 s:9.1.1 MFC Command Opcode Channel] the command parameter is the word's low 16 bits, whose upper 8 bits are reserved.
    /// [CBEA p:57 s:7.1.3] those reserved bits belong to the opcode: the reserved commands are x'8000' to x'FFFF'.
    #[inline]
    pub const fn opcode(self) -> u32 {
        self.0 & 0xFFFF
    }

    /// The class of [`Self::opcode`].
    #[inline]
    pub const fn class(self) -> super::spu_mfc::MfcOpcodeClass {
        super::spu_mfc::mfc_opcode_class(self.opcode() as u16)
    }

    /// Transfer class id, which steers bus bandwidth.
    ///
    /// [CBE-Handbook p:457 s:17.9.6 MFC Class ID and MFC Command Opcode Channel] TclassID steers how large a share of the bus a transfer is given.
    #[inline]
    pub const fn tclass_id(self) -> u8 {
        (self.0 >> 24) as u8
    }

    /// Replacement class id, which steers L2-cache and TLB replacement.
    ///
    /// [CBE-Handbook p:457 s:17.9.6 MFC Class ID and MFC Command Opcode Channel] RclassID steers which L2-cache and address-translation entries are chosen for replacement.
    #[inline]
    pub const fn rclass_id(self) -> u8 {
        (self.0 >> 16) as u8
    }

    /// True where the word's opcode is in the reserved range.
    ///
    /// [CBEA p:113 s:9.1.1 MFC Command Opcode Channel] the command parameter is the word's low halfword, whose own leading bit marks the opcode reserved.
    #[inline]
    pub const fn names_a_reserved_opcode(self) -> bool {
        matches!(self.class(), super::spu_mfc::MfcOpcodeClass::Reserved)
    }
}

/// `SPU_In_Mbox`'s offset in an SPE's problem-state area.
///
/// [CBEA p:99 s:8.6.2] SPU_In_Mbox is at offset x'0400C' of the SPE's problem-state area.
pub const SPU_IN_MBOX_OFFSET: u32 = 0x0400C;
/// `SPU_Sig_Notify_1`'s offset in an SPE's problem-state area.
///
/// [CBEA p:102 s:8.7.1] SPU_Sig_Notify_1 is at offset x'1400C' of the SPE's problem-state area.
pub const SPU_SIG_NOTIFY_1_OFFSET: u32 = 0x1400C;
/// `SPU_Sig_Notify_2`'s offset in an SPE's problem-state area.
///
/// [CBEA p:103 s:8.7.2] SPU_Sig_Notify_2 is at offset x'1C00C' of the SPE's problem-state area.
pub const SPU_SIG_NOTIFY_2_OFFSET: u32 = 0x1C00C;
/// SPU local store size in bytes (256 KiB).
///
/// [CBE-Handbook p:64 s:3.1.1] Local Store is a 256 KB single-ported memory.
pub const SPU_LS_SIZE: usize = 256 * 1024;

/// The local storage limit register value that selects all of
/// [`SPU_LS_SIZE`].
///
/// [SPU-ISA p:31 s:3 Table 3-1] the LSLR is 2^n - 1 for an effective size of 2^n bytes, and 0x0003FFFF selects 256 KB.
pub const SPU_LSLR_FULL: u32 = SPU_LS_SIZE as u32 - 1;

/// Entries in the MFC SPU command queue: the MFC_Cmd channel's count on
/// an empty queue.
///
/// [CBE-Handbook p:445 s:17.1 Table 17-2] MFC_Cmd has 16 maximum entries; [CBE-Handbook p:528 s:19.4.3.2] each MFC has a 16-entry SPU command queue.
pub const MFC_SPU_QUEUE_DEPTH: u32 = 16;

/// The largest MFC transfer, and the largest list, in bytes.
///
/// [CBEA p:57 s:7.2 Table 7-6] a transfer size or a list transfer size greater than 16K bytes is an alignment error.
pub const MFC_TRANSFER_SIZE_MAX: u32 = 0x4000;

/// The reserved bits of the MFC_Size channel, bits 0:16.
///
/// [CBEA p:116 s:9.1.4] MFC_Size bits 0:16 are reserved; the transfer size is bits 17:31.
pub const MFC_SIZE_RESERVED_MASK: u32 = 0xFFFF_8000;

/// The reserved bits of the MFC_TagID channel, bits 0:26.
///
/// [CBEA p:115 s:9.1.3] MFC_TagID bits 0:26 are reserved; the tag is bits 27:31.
pub const MFC_TAG_ID_RESERVED_MASK: u32 = 0xFFFF_FFE0;

/// The low four address bits a transfer's local-store and effective
/// addresses must share.
///
/// [CBEA p:57 s:7.2 Table 7-6] bits 60:63 of the effective address must equal LSA bits 28:31 for every put and get and for sndsig.
pub const MFC_ADDRESS_LOW_BITS: u32 = 0xF;

/// The low bits of a list address that must be zero: a list is
/// doubleword aligned.
///
/// [CBEA p:57 s:7.2 Table 7-6] bits 29:31 of the list address must be 000.
pub const MFC_LIST_ADDRESS_LOW_BITS: u32 = 0x7;

/// The one transfer size a sndsig command takes.
///
/// [CBEA p:57 s:7.2 Table 7-6] a sndsig transfer size other than 4 bytes is an alignment error.
pub const MFC_SNDSIG_SIZE: u32 = 4;

/// The class 0 interrupt status bit for a DMA alignment error, bit 63
/// of the 64-bit register.
///
/// [CBEA p:274 s:21.7.1] INT_Stat_class0 bit 63 (A) is the DMA alignment interrupt, bit 62 (C) the invalid DMA command interrupt.
pub const MFC_CLASS0_ALIGNMENT: u64 = 1;

/// The class 0 interrupt status bit for an invalid DMA command, bit 62
/// of the 64-bit register.
pub const MFC_CLASS0_INVALID_COMMAND: u64 = 1 << 1;

/// Entries in the SPU inbound mailbox.
///
/// [CBE-Handbook p:445 s:17.1 Table 17-2] SPU_RdInMbox has 4 maximum entries.
pub const SPU_IN_MBOX_DEPTH: u32 = 4;

/// Entries in the SPU outbound mailbox.
///
/// [CBE-Handbook p:445 s:17.1 Table 17-2] SPU_WrOutMbox has 1 maximum entry.
pub const SPU_OUT_MBOX_DEPTH: u32 = 1;

/// Entries in the SPU outbound interrupt mailbox.
///
/// [CBE-Handbook p:445 s:17.1 Table 17-2] SPU_WrOutIntrMbox has 1 maximum entry.
pub const SPU_OUT_INTR_MBOX_DEPTH: u32 = 1;

/// Mask of the stop-and-signal code: the low 14 bits of a `stop` word.
///
/// [SPU-ISA p:238 s:10] stop carries its signal type in bits 18:31.
pub const SPU_STOP_CODE_MASK: u32 = 0x3FFF;

/// The stop code a `stopd` reports, whatever its operand fields hold.
///
/// [CBEA p:93 s:8.5.2] a stopd always sets the StopCode field to x'3FFF'.
pub const SPU_STOPD_CODE: u16 = 0x3FFF;

/// Shift of the `SPU_Status` StopCode field, which holds bits 0:15 of
/// the big-endian word.
///
/// [CBEA p:93 s:8.5.2] StopCode is bits 0:15; a stop's 14-bit code lands in bits 2:15.
pub const SPU_STATUS_STOP_CODE_SHIFT: u32 = 16;

/// `SPU_Status` C: an invalid channel instruction stopped the SPU (bit 25).
///
/// [CBEA p:93 s:8.5.2] bit 25 C: invalid channel instruction detected, SPU halted.
pub const SPU_STATUS_C: u32 = 1 << (31 - 25);

/// `SPU_Status` I: an invalid instruction stopped the SPU (bit 26).
///
/// [CBEA p:93 s:8.5.2] bit 26 I: invalid instruction detected, SPU halted.
pub const SPU_STATUS_I: u32 = 1 << (31 - 26);

/// `SPU_Status` W: the SPU stopped while waiting on a blocked channel
/// (bit 28).
///
/// [CBEA p:94 s:8.5.2] bit 28 W: SPU waiting on a blocked channel, set with the stopped status when the PPE stops a waiting SPU.
pub const SPU_STATUS_W: u32 = 1 << (31 - 28);

/// `SPU_Status` H: a halt instruction stopped the SPU (bit 29).
///
/// [CBEA p:94 s:8.5.2] bit 29 H: SPU halted due to a halt instruction.
pub const SPU_STATUS_H: u32 = 1 << (31 - 29);

/// `SPU_Status` P: a stop or stopd stopped the SPU (bit 30).
///
/// [CBEA p:94 s:8.5.2] bit 30 P: SPU stopped due to a stop-and-signal instruction, stop or stopd.
pub const SPU_STATUS_P: u32 = 1 << (31 - 30);

/// `SPU_Status` R: the SPU is running (bit 31, the least significant
/// bit).
///
/// [CBEA p:94 s:8.5.2] bit 31 R: 0 SPU stopped or halted, 1 SPU running.
pub const SPU_STATUS_R: u32 = 1;

/// Number of SPU general-purpose 128-bit registers (r0..r127).
///
/// [SPU-ISA p:25 s:2] The SPU architecture defines 128 general-purpose
/// registers, each holding 128 data bits.
pub const SPU_REG_COUNT: usize = 128;
