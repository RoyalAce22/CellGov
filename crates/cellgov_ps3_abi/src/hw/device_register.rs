//! The words of a system bus device's MMIO registers that a source gives,
//! each for the loads of one instruction of the kernel.
//!
//! A load of a device register gives the device's state. CellGov models
//! no device state. A register word therefore answers only the one
//! instruction whose load the source covers. Every other access to a
//! device's MMIO stops.

/// A word of a device's MMIO region that a source gives for the loads of
/// one kernel instruction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceRegisterRead {
    /// The firmware version, such as `"4.93"`.
    pub firmware: &'static str,
    /// The bus id of the region's device.
    pub bus: u64,
    /// The device id of the region's device.
    pub device: u64,
    /// The bus address of the region, as
    /// [`crate::lv1::device_bus::MMIO_REGIONS`] gives it.
    pub region: u64,
    /// The offset of the word in the region.
    pub offset: u64,
    /// The word that the load gets. Each bit outside `known` is 0.
    pub value: u32,
    /// The bits of the word that the source gives.
    pub known: u32,
    /// The effective address of the instruction whose load gets the word.
    /// The instruction's code keeps only the bits of `known`.
    pub reader: u64,
}

/// The register words that the 4.93 kernel reads from the MMIO region of
/// device 1 of bus 1 at the bus address 0x3810000, before its first store
/// to that region.
///
/// The function at [OFW-4.93 lv2_kernel a:0x8000000000245818] reads the
/// two words. It keeps only these bits:
/// - the low byte of the word at +0, as the offset of the registers that
///   it accesses next
///   [OFW-4.93 lv2_kernel a:0x8000000000245914-0x8000000000245924];
/// - the low 4 bits of the word at +4, which it refuses at 0
///   [OFW-4.93 lv2_kernel a:0x8000000000245918-0x800000000024592C].
///
/// The capture [`DEVICE_REGISTER_CAPTURE`] reads the kernel's copies of
/// those bits on the reference console: 0x10 and 2.
pub const DEVICE_REGISTER_READS: &[DeviceRegisterRead] = &[
    DeviceRegisterRead {
        firmware: "4.93",
        bus: 1,
        device: 1,
        region: 0x381_0000,
        offset: 0,
        value: 0x10,
        known: 0xFF,
        // [OFW-4.93 lv2_kernel a:0x800000000024590C] lwz r0,0(r11)
        reader: 0x8000_0000_0024_590C,
    },
    DeviceRegisterRead {
        firmware: "4.93",
        bus: 1,
        device: 1,
        region: 0x381_0000,
        offset: 4,
        value: 2,
        known: 0xF,
        // [OFW-4.93 lv2_kernel a:0x8000000000245910] lwz r9,4(r11)
        reader: 0x8000_0000_0024_5910,
    },
];

/// The console capture of the kernel's copies of the register words of
/// devices 1 and 2 of bus 1.
///
/// The path is relative to the workspace root. Its README gives the
/// method and the analysis.
pub const DEVICE_REGISTER_CAPTURE: &str = "tests/micro/lv2_device_register_copies";

#[cfg(test)]
#[path = "tests/device_register_console_tests.rs"]
mod console_tests;
