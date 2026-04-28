//! cellGcmSys PS3 ABI: error codes and RSX-local memory region constants.
//!
//! Mirrors the layout of RPCS3's `rpcs3/Emu/Cell/Modules/cellGcmSys.h`.
//! Behaviour (handlers, dispatch, allocation) lives in
//! `cellgov_core::hle::cellGcmSys`; this module is data only.

/// `CELL_GCM_ERROR_*` band (`0x8021_00xx`).
pub mod error {
    /// `cellGcmAddressToOffset` failure code. The GCM module returns
    /// this single value across every "address is not mappable"
    /// condition.
    pub const FAILURE: u32 = 0x8021_00ff;
}

/// RSX-local memory region in PS3 VA space. Addresses
/// `[BASE, BASE + SIZE)` translate to RSX-side offsets `[0, SIZE)`.
pub mod rsx_local {
    /// Base of the RSX-local region.
    pub const BASE: u32 = 0xC000_0000;
    /// Size of the RSX-local region.
    pub const SIZE: u32 = 0x1000_0000;
}
