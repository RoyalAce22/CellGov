//! The numbers of the storage device hypercalls.

/// `storage_read`: six inputs and one output.
/// - r3 is the id of a device of the storage bus, or -1 for the
///   notification device.
/// - r4 is the id of a region of that device.
/// - r5 is the start sector.
/// - r6 is the sector count.
/// - r7 is the flags.
/// - r8 is the logical partition address of the buffer.
/// - The output r4 is the tag of the request.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x308B88] reads the inputs in this
/// form [OFW-4.93 lv1 a:0x308B98-0x308BD4]; see
/// [`crate::lv1::storage_read`].
pub const STORAGE_READ: u64 = 245;

/// `storage_write`: six inputs and one output.
/// - r3 is the id of a device of the storage bus, or -1 for the
///   notification device.
/// - r4 is the id of a region of that device.
/// - r5 is the start sector.
/// - r6 is the sector count.
/// - r7 is the flags.
/// - r8 is the logical partition address of the data.
/// - The output r4 is the tag of the request.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x308A08] reads the inputs in this
/// form [OFW-4.93 lv1 a:0x308A18-0x308A54]; see [`crate::lv1::storage`].
pub const STORAGE_WRITE: u64 = 246;

/// `storage_get_async_status`: one input and two outputs.
/// - r3 is the id of a device of the storage bus, or -1 for the
///   notification device.
/// - The output r4 is the tag of a completed request, or -1 for the
///   word of the partition's open slot.
/// - The output r5 is the status of that request, or that word.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x3093AC] reads r3 at
/// [OFW-4.93 lv1 a:0x3093C0]; see [`crate::lv1::storage_status`].
pub const STORAGE_GET_ASYNC_STATUS: u64 = 249;
