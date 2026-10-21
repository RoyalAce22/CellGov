//! MFC transfer times a retail console measures, in time-base ticks.
//!
//! Every value is the `spu_dma_timing` capture's (CECH-2001A, 4.93 CEX,
//! `tests/micro/spu_dma_timing/ps3/cech20-cex-493/`): the sum of
//! [`MFC_TIMING_RUNS`] runs, each read from the SPU decrementer from the
//! command's issue to its tag group's completion. The decrementer runs at
//! the time-base frequency, 79.8 MHz. The test
//! `crates/cellgov_spu/tests/it/console_dma_timing.rs` holds each table
//! to the committed capture.

/// The runs each sum covers (payload offset 16).
pub const MFC_TIMING_RUNS: u32 = 32;

/// The transfer sizes of each table: 16 bytes doubled ten times, to
/// 16 KB, the largest single transfer.
pub const MFC_TIMING_SIZES: [u32; 11] = [16, 32, 64, 128, 256, 512, 1024, 2048, 4096, 8192, 16384];

/// A tag-status wait with nothing queued (payload offset 20). Each table
/// entry includes one such wait.
pub const MFC_TAG_WAIT_TB_SUM: u32 = 64;

/// The line the off-line tables place both addresses 16 bytes past.
pub const MFC_TIMING_LINE_BYTES: u64 = 128;

/// A get with both addresses on a 128-byte line (payload offset 32).
pub const MFC_GET_LINE_TB_SUM: [u32; 11] =
    [450, 444, 443, 436, 447, 492, 526, 626, 1037, 1784, 3264];

/// A get with both addresses 16 bytes past a 128-byte line (payload
/// offset 76).
pub const MFC_GET_OFF_LINE_TB_SUM: [u32; 11] =
    [441, 431, 433, 449, 469, 490, 557, 783, 1136, 1825, 3309];

/// A put with both addresses on a 128-byte line (payload offset 120).
pub const MFC_PUT_LINE_TB_SUM: [u32; 11] =
    [282, 277, 277, 278, 310, 327, 382, 476, 704, 1160, 2075];

/// A put with both addresses 16 bytes past a 128-byte line (payload
/// offset 164).
pub const MFC_PUT_OFF_LINE_TB_SUM: [u32; 11] =
    [277, 280, 282, 305, 319, 342, 387, 491, 727, 1182, 2094];

/// A getllar with the read of its atomic status that follows (payload
/// offset 220). No tag wait is in it.
pub const MFC_GETLLAR_TB_SUM: u32 = 172;

/// A putllc after a getllar of its line, with the read of its atomic
/// status that follows (payload offset 224). Every run kept its
/// reservation.
pub const MFC_PUTLLC_TB_SUM: u32 = 172;

/// A putlluc with the read of its atomic status that follows (payload
/// offset 228).
pub const MFC_PUTLLUC_TB_SUM: u32 = 153;

/// The bytes a time-base tick that every SPU's transfers share, fitted
/// to the `spu_dma_contention` capture.
///
/// One SPU's 16 KB get beside 2 to 5 SPUs streaming 16 KB gets takes the
/// latency of a get alone plus its share of 323 to 338 bytes a tick.
/// This is the share beside five streamers. One SPU alone moves a get's
/// bytes at about 177 a tick and a put's at about 286, both under it.
pub const MFC_SHARED_BYTES_PER_TB: u64 = 327;

/// The least time one transfer holds its MFC's queue, summed over
/// [`MFC_TIMING_RUNS`] runs.
///
/// Each 128-byte element of a list of up to sixteen adds 1.43 ticks in
/// the `spu_dma_contention` capture, against the 0.72 its bytes take at
/// a get's rate.
pub const MFC_TRANSFER_MIN_TB_SUM: u64 = 46;
