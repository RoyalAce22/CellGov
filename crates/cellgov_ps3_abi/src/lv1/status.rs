//! LV1 status codes: the value a hypercall returns in r3.
//!
//! Source: Linux `arch/powerpc/include/asm/ps3.h`, `enum lv1_result`.
//! The enum marks -1 and -12 as not used.

/// The call succeeded.
pub const LV1_SUCCESS: i64 = 0;
/// `LV1_RESOURCE_SHORTAGE`.
pub const LV1_RESOURCE_SHORTAGE: i64 = -2;
/// `LV1_NO_PRIVILEGE`.
pub const LV1_NO_PRIVILEGE: i64 = -3;
/// `LV1_DENIED_BY_POLICY`.
pub const LV1_DENIED_BY_POLICY: i64 = -4;
/// `LV1_ACCESS_VIOLATION`.
pub const LV1_ACCESS_VIOLATION: i64 = -5;
/// `LV1_NO_ENTRY`.
pub const LV1_NO_ENTRY: i64 = -6;
/// `LV1_DUPLICATE_ENTRY`.
pub const LV1_DUPLICATE_ENTRY: i64 = -7;
/// `LV1_TYPE_MISMATCH`.
pub const LV1_TYPE_MISMATCH: i64 = -8;
/// `LV1_BUSY`.
pub const LV1_BUSY: i64 = -9;
/// `LV1_EMPTY`.
pub const LV1_EMPTY: i64 = -10;
/// `LV1_WRONG_STATE`.
pub const LV1_WRONG_STATE: i64 = -11;
/// `LV1_NO_MATCH`.
pub const LV1_NO_MATCH: i64 = -13;
/// `LV1_ALREADY_CONNECTED`.
pub const LV1_ALREADY_CONNECTED: i64 = -14;
/// `LV1_UNSUPPORTED_PARAMETER_VALUE`.
pub const LV1_UNSUPPORTED_PARAMETER_VALUE: i64 = -15;
/// `LV1_CONDITION_NOT_SATISFIED`.
pub const LV1_CONDITION_NOT_SATISFIED: i64 = -16;
/// `LV1_ILLEGAL_PARAMETER_VALUE`.
pub const LV1_ILLEGAL_PARAMETER_VALUE: i64 = -17;
/// `LV1_BAD_OPTION`.
pub const LV1_BAD_OPTION: i64 = -18;
/// `LV1_IMPLEMENTATION_LIMITATION`.
pub const LV1_IMPLEMENTATION_LIMITATION: i64 = -19;
/// `LV1_NOT_IMPLEMENTED`.
///
/// The enum gives the name only. No public source says that lv1 returns
/// it for a number it has no handler for.
pub const LV1_NOT_IMPLEMENTED: i64 = -20;
/// `LV1_INVALID_CLASS_ID`.
pub const LV1_INVALID_CLASS_ID: i64 = -21;
/// `LV1_CONSTRAINT_NOT_SATISFIED`.
pub const LV1_CONSTRAINT_NOT_SATISFIED: i64 = -22;
/// `LV1_ALIGNMENT_ERROR`.
pub const LV1_ALIGNMENT_ERROR: i64 = -23;
/// `LV1_HARDWARE_ERROR`.
pub const LV1_HARDWARE_ERROR: i64 = -24;
/// `LV1_INVALID_DATA_FORMAT`.
pub const LV1_INVALID_DATA_FORMAT: i64 = -25;
/// `LV1_INVALID_OPERATION`.
pub const LV1_INVALID_OPERATION: i64 = -26;
/// `LV1_INTERNAL_ERROR`.
pub const LV1_INTERNAL_ERROR: i64 = -32768;
