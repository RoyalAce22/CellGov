//! The numbers of the storage device hypercalls.

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
