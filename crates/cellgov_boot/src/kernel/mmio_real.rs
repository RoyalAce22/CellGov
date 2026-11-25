//! The real address of each MMIO region that a kernel run declares, and
//! the refusal when lv1's address function gives no address that CellGov
//! holds.
//!
//! Each real address comes from
//! [`cellgov_ps3_abi::lv1::device_bus::real_address_in`] over the rows
//! of [`MmioTables`]. When one region has no real address, the run
//! declares none of the device MMIO.

use cellgov_mem::MemError;
use cellgov_ps3_abi::hw::device_register::{DeviceRegisterRead, DEVICE_REGISTER_READS};
use cellgov_ps3_abi::hw::ehci::{EhciController, EHCI_CONTROLLERS};
use cellgov_ps3_abi::lv1::device_bus::{
    real_address_in, MmioDeviceIndex, MmioRealAddress, MmioRegion, MMIO_BUS_DOUBLEWORD_INDEX,
    MMIO_DEVICE_INDEXES, MMIO_REGIONS,
};

/// The tables that give the device MMIO of a kernel run.
#[derive(Debug, Clone, Copy)]
pub(super) struct MmioTables<'a> {
    /// The MMIO regions of the devices.
    pub(super) regions: &'a [MmioRegion],
    /// The index of each device.
    pub(super) indexes: &'a [MmioDeviceIndex],
    /// The register words that a source gives.
    pub(super) reads: &'a [DeviceRegisterRead],
    /// The EHCI controllers.
    pub(super) controllers: &'a [EhciController],
}

/// The tables of the crate `cellgov_ps3_abi`, which a kernel run uses.
pub(super) const SHIPPED: MmioTables<'static> = MmioTables {
    regions: MMIO_REGIONS,
    indexes: MMIO_DEVICE_INDEXES,
    reads: DEVICE_REGISTER_READS,
    controllers: EHCI_CONTROLLERS,
};

impl MmioTables<'_> {
    /// The real address of the region at `bus_address` of device `device`
    /// of bus `bus` at `firmware`.
    ///
    /// # Errors
    ///
    /// Returns [`DeviceMmioError::NoRealAddress`] when lv1's address
    /// function gives no real address that CellGov holds.
    pub(super) fn real(
        &self,
        firmware: &str,
        bus: u64,
        device: u64,
        bus_address: u64,
    ) -> Result<u64, DeviceMmioError> {
        let refusal = match real_address_in(
            self.regions,
            self.indexes,
            firmware,
            bus,
            device,
            bus_address,
        ) {
            MmioRealAddress::Real(real) => return Ok(real),
            MmioRealAddress::NoRegion => MmioRealRefusal::NoRegion,
            MmioRealAddress::NoIndex => MmioRealRefusal::NoIndex,
            MmioRealAddress::BusDoubleword => MmioRealRefusal::BusDoubleword,
        };
        Err(DeviceMmioError::NoRealAddress {
            firmware: firmware.to_owned(),
            bus,
            device,
            bus_address,
            source: refusal,
        })
    }
}

/// Why a kernel run has no real address for an MMIO region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum MmioRealRefusal {
    /// The device has no region at the bus address.
    #[error("the device has no MMIO region at that bus address")]
    NoRegion,
    /// No lv1 image read shows the index of the device.
    #[error("no lv1 image read shows the index of the device")]
    NoIndex,
    /// The device has the index for which lv1 also adds a doubleword of
    /// the bus object. CellGov does not hold that doubleword.
    #[error(
        "lv1 adds a doubleword of the bus object for device index {}, and CellGov does not hold it",
        MMIO_BUS_DOUBLEWORD_INDEX
    )]
    BusDoubleword,
}

/// Why a kernel run did not declare the MMIO of the devices.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DeviceMmioError {
    /// A region, a register word or an EHCI controller has no real
    /// address. The run declares none of the device MMIO.
    #[error(
        "bus address 0x{bus_address:016x} of device {device} of bus {bus} at firmware \
         {firmware} has no real address: {source}"
    )]
    NoRealAddress {
        /// The firmware version.
        firmware: String,
        /// The bus id of the device.
        bus: u64,
        /// The device id of the device.
        device: u64,
        /// The bus address of the region.
        bus_address: u64,
        /// Why the region has no real address.
        source: MmioRealRefusal,
    },
    /// An EHCI controller is at a firmware for which CellGov has no
    /// register rule. The run declares none of the device MMIO.
    #[error(
        "the EHCI controller of device {device} of bus {bus} at firmware {firmware} has no \
         register rule"
    )]
    NoEhciRule {
        /// The firmware version.
        firmware: String,
        /// The bus id of the controller's device.
        bus: u64,
        /// The device id of the controller's device.
        device: u64,
    },
    /// The real-mode address space refused a declaration.
    #[error("real-mode kernel address space: {0}")]
    Memory(#[from] MemError),
}
