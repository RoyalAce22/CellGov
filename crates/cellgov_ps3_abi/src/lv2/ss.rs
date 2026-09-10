//! `sys_ss_*` ABI constants: the status codes the secure-services
//! syscalls answer with.

/// Status `sys_ss_access_control_engine` answers for a `pkg_id` other
/// than 1, 2 or 3.
///
/// The value sits in the SS error domain, not in the LV2 errno block
/// of [`crate::lv2::errno`]. Every call site in the installed firmware
/// loads 1, 2 or 3, so no witness reaches this status and its symbol is
/// unestablished; the name is CellGov's.
pub const SS_ACCESS_CONTROL_UNKNOWN_PKG_ID: u32 = 0x8001_051D;
