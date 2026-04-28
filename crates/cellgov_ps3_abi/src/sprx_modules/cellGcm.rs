//! cellGcm / NVIDIA RSX command-stream PS3 ABI: NV header flags,
//! method codes, flip status enum, label-region geometry.
//!
//! Behaviour (the FIFO parser, the dispatch registry, the
//! commit-boundary semantics, the `RsxFlipState` machine) lives in
//! `cellgov_core::rsx`; this module is data only.

// CellGcmDisplayFlipStatus byte values written to the flip-status slot
// in guest-visible RSX driver-info memory.

/// Flip status byte value: no flip pending or prior flip completed.
pub const CELL_GCM_DISPLAY_FLIP_STATUS_DONE: u8 = 0;

/// Flip status byte value: flip issued, not yet complete.
pub const CELL_GCM_DISPLAY_FLIP_STATUS_WAITING: u8 = 1;

// NVIDIA RSX command header bit flags / shifts / masks.

/// Header bit: arguments do NOT increment the method register.
pub const NV_FLAG_NON_INCREMENT: u32 = 0x4000_0000;

/// Header bit: this header is a JUMP (FIFO control transfer).
pub const NV_FLAG_JUMP: u32 = 0x2000_0000;

/// Header bit: this header is a CALL (FIFO subroutine call).
pub const NV_FLAG_CALL: u32 = 0x0000_0002;

/// Header bit: this header is a RETURN (FIFO subroutine return).
pub const NV_FLAG_RETURN: u32 = 0x0002_0000;

/// Header bit: this header is a NEW-style JUMP (longer offset field).
pub const NV_FLAG_NEW_JUMP: u32 = 0x0000_0001;

/// Bit position of the per-header argument count.
pub const NV_COUNT_SHIFT: u32 = 18;

/// 11-bit mask for the per-header argument count.
pub const NV_COUNT_MASK_11: u32 = 0x7FF;

/// Mask for the method-id bits in a header.
pub const NV_METHOD_MASK: u32 = 0x0000_FFFC;

/// Offset mask for old-style JUMP headers.
pub const NV_OLD_JUMP_OFFSET_MASK: u32 = 0x1FFF_FFFC;

/// Offset mask for new-style JUMP headers.
pub const NV_NEW_JUMP_OFFSET_MASK: u32 = 0xFFFF_FFFC;

/// Offset mask for CALL headers.
pub const NV_CALL_OFFSET_MASK: u32 = 0x1FFF_FFFC;

// NV406E (Channel command engine) method codes.

/// `NV406E_SET_REFERENCE`: writes the FIFO reference register.
pub const NV406E_SET_REFERENCE: u16 = 0x0050;

/// `NV406E_SEMAPHORE_OFFSET`: sets the semaphore-write target offset.
pub const NV406E_SEMAPHORE_OFFSET: u16 = 0x0064;

/// `NV406E_SEMAPHORE_ACQUIRE`: blocks the FIFO until the labelled
/// semaphore matches.
pub const NV406E_SEMAPHORE_ACQUIRE: u16 = 0x0068;

/// `NV406E_SEMAPHORE_RELEASE`: writes the labelled semaphore.
pub const NV406E_SEMAPHORE_RELEASE: u16 = 0x006C;

// NV4097 (3D engine) method codes.

/// `NV4097_NO_OPERATION`: header-only filler.
pub const NV4097_NO_OPERATION: u16 = 0x0100;

/// `NV4097_GET_REPORT`: writes a report record at `reports_base +
/// (offset & NV4097_REPORT_OFFSET_MASK)`.
pub const NV4097_GET_REPORT: u16 = 0x1800;

/// `NV4097_SET_SEMAPHORE_OFFSET`: sets the back-end semaphore target.
pub const NV4097_SET_SEMAPHORE_OFFSET: u16 = 0x1D6C;

/// `NV4097_BACK_END_WRITE_SEMAPHORE_RELEASE`: writes the back-end
/// semaphore at the previously-set offset.
pub const NV4097_BACK_END_WRITE_SEMAPHORE_RELEASE: u16 = 0x1D70;

/// `NV4097_GET_REPORT` offset-field mask.
pub const NV4097_REPORT_OFFSET_MASK: u32 = 0xFFFF_FFFF;

/// `cellGcm` flip command code: writes a buffer-id arg to trigger a
/// flip on the next commit boundary.
pub const GCM_FLIP_COMMAND: u16 = 0xFEAC;

// Reports-region label addressing.

/// Bytes between consecutive labels in the reports semaphore region.
pub const LABEL_STRIDE: u32 = 0x10;

/// Number of addressable labels.
pub const LABEL_COUNT: u32 = 256;
