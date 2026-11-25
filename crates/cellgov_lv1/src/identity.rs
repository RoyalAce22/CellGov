//! The one synthetic console identity that CellGov serves.
//!
//! On the console, lv1 gives the kernel data that it derives from the
//! console's identity, and each console gives different bytes. CellGov
//! serves one fixed synthetic identity in their place, so a run
//! reproduces on every machine. The identity holds only the fields that
//! the kernel tests. No field is the value of one real console.

use cellgov_ps3_abi::lv1::uart::{
    DISPATCHER_IDENTITY_DATA_BYTES, IDENTITY_DATA_PRODUCT_CODE_OFFSET,
    IDENTITY_DATA_PRODUCT_SUB_CODE_OFFSET,
};

/// The fields of a console identity that CellGov serves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ConsoleIdentity {
    /// The product code; see [`IDENTITY_DATA_PRODUCT_CODE_OFFSET`].
    pub(crate) product_code: u16,
    /// The product sub code; see [`IDENTITY_DATA_PRODUCT_SUB_CODE_OFFSET`].
    pub(crate) product_sub_code: u16,
}

impl ConsoleIdentity {
    /// The data bytes of the reply to request 0x19004 of service 0x19000.
    ///
    /// The two codes are big-endian halfwords at their offsets. No source
    /// gives the format of the halfwords at +0 and +6. CellGov knows of no
    /// kernel test of them, and gives 0 in them.
    pub(crate) const fn dispatcher_data(&self) -> [u8; DISPATCHER_IDENTITY_DATA_BYTES] {
        let mut data = [0; DISPATCHER_IDENTITY_DATA_BYTES];
        let code = self.product_code.to_be_bytes();
        let sub_code = self.product_sub_code.to_be_bytes();
        data[IDENTITY_DATA_PRODUCT_CODE_OFFSET] = code[0];
        data[IDENTITY_DATA_PRODUCT_CODE_OFFSET + 1] = code[1];
        data[IDENTITY_DATA_PRODUCT_SUB_CODE_OFFSET] = sub_code[0];
        data[IDENTITY_DATA_PRODUCT_SUB_CODE_OFFSET + 1] = sub_code[1];
        data
    }
}

/// The synthetic console identity that CellGov serves.
///
/// Each value is a CellGov choice:
/// - The product code is 0x84, the last data byte of the device type that
///   CellGov serves for request 0x19002. So the two replies agree.
/// - The product sub code is 8, a value in the range 1 to 12 that the
///   kernel tests.
pub(crate) const SYNTHETIC_CONSOLE_IDENTITY: ConsoleIdentity = ConsoleIdentity {
    product_code: 0x84,
    product_sub_code: 8,
};

#[cfg(test)]
#[path = "tests/identity_tests.rs"]
mod tests;
