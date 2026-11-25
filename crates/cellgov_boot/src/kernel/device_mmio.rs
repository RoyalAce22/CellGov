//! The MMIO regions of the system bus devices, and the stop of an access
//! to one.
//!
//! The console has storage at each region's real address: the device's
//! registers. CellGov models few of them, so the run declares each region
//! an unmodelled span. A span holds no bytes, so it never reads as zeros,
//! and a store there never lands. An access there stops the run with the
//! device that holds the address, unless one of these words answers it:
//! - A source gives some register words for the loads of one kernel
//!   instruction each ([`DEVICE_REGISTER_READS`]). The run declares each
//!   such word a sourced word of its span, so that instruction's load
//!   gets the word.
//! - The run models some operational registers and the PS3 setup
//!   registers of each EHCI controller as register words ([`ehci`]).
//!
//! [`DEVICE_REGISTER_READS`]: cellgov_ps3_abi::hw::device_register::DEVICE_REGISTER_READS

use cellgov_effects::FaultKind;
use cellgov_exec::ExecutionStepResult;
use cellgov_mem::GuestMemory;
use cellgov_ppu::FAULT_UNMODELLED_STORAGE;
use cellgov_ps3_abi::hw::device_register::DeviceRegisterRead;
use cellgov_ps3_abi::lv1::device_bus::MmioRegion;

pub use mmio_real::{DeviceMmioError, MmioRealRefusal};
use mmio_real::{MmioTables, SHIPPED};

#[cfg(test)]
#[path = "tests/device_mmio_register_tests.rs"]
mod register_tests;

#[cfg(test)]
#[path = "tests/device_mmio_real_tests.rs"]
mod real_tests;

#[path = "ehci.rs"]
mod ehci;

#[path = "mmio_real.rs"]
mod mmio_real;

#[cfg(test)]
#[path = "tests/ehci_tests.rs"]
mod ehci_tests;

#[cfg(test)]
#[path = "tests/ehci_setup_tests.rs"]
mod ehci_setup_tests;

/// An access of a hardware thread to the MMIO of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceMmioAccess {
    /// The PPE hardware thread that made the access.
    pub thread: u8,
    /// The address of the instruction that made the access, when the
    /// unit named it.
    pub pc: Option<u64>,
    /// The MMIO region that holds the access.
    pub region: MmioRegion,
    /// The bus address of the first byte that the access reaches.
    pub bus_address: u64,
    /// The real address of that byte.
    pub real: u64,
    /// The effective address of that byte.
    pub ea: u64,
}

/// The MMIO regions of `tables` at `firmware`, each with its real address
/// or the refusal of that address.
fn regions_of<'a>(
    tables: MmioTables<'a>,
    firmware: &'a str,
) -> impl Iterator<Item = Result<(u64, &'a MmioRegion), DeviceMmioError>> + 'a {
    tables
        .regions
        .iter()
        .filter(move |region| region.firmware == firmware)
        .map(move |region| {
            let real = tables.real(firmware, region.bus, region.device, region.address)?;
            Ok((real, region))
        })
}

/// The register words of `tables` at `firmware`, each with its real
/// address.
///
/// # Errors
///
/// Returns [`DeviceMmioError::NoRealAddress`] for the first word whose
/// region has no real address.
fn words_of<'a>(
    tables: MmioTables<'a>,
    firmware: &str,
) -> Result<Vec<(u64, &'a DeviceRegisterRead)>, DeviceMmioError> {
    tables
        .reads
        .iter()
        .filter(|read| read.firmware == firmware)
        .map(|read| {
            let real = tables.real(firmware, read.bus, read.device, read.region)?;
            Ok((real + read.offset, read))
        })
        .collect()
}

/// Declares each MMIO region of the devices at `firmware` an unmodelled
/// span of `mem`. In those spans it declares these words:
/// - each register word that a source gives at `firmware`, as a sourced
///   word;
/// - some operational registers and the PS3 setup registers of each EHCI
///   controller at `firmware`, as register words.
///
/// # Errors
///
/// Returns one of these:
/// - [`DeviceMmioError::NoRealAddress`] when a region, a word or a
///   controller has no real address. The function then declares nothing;
/// - the refusal of [`GuestMemory::declare_unmodelled`] when a region
///   overlaps a region of `mem` or another span, or runs past the end of
///   the address space;
/// - the refusal of [`GuestMemory::declare_sourced_word`] when a word is
///   not an aligned word of one span, or repeats a word and reader;
/// - the refusal of a register file, reader or writer declaration.
pub(super) fn declare_device_mmio(
    mem: &mut GuestMemory,
    firmware: &str,
) -> Result<(), DeviceMmioError> {
    declare_from(mem, SHIPPED, firmware)
}

/// Declares the device MMIO of `tables` at `firmware` in `mem`, as
/// [`declare_device_mmio`] does.
fn declare_from(
    mem: &mut GuestMemory,
    tables: MmioTables<'_>,
    firmware: &str,
) -> Result<(), DeviceMmioError> {
    let regions: Vec<_> = regions_of(tables, firmware).collect::<Result<_, _>>()?;
    let words = words_of(tables, firmware)?;
    let controllers = ehci::controllers_of(tables, firmware)?;
    for (real, region) in regions {
        mem.declare_unmodelled(real, region.size)?;
    }
    for (real, read) in words {
        mem.declare_sourced_word(real, read.value, read.reader)?;
    }
    ehci::declare_ehci(mem, firmware, &controllers)?;
    Ok(())
}

/// The MMIO region of `tables` at `firmware` that holds the real address
/// `real`, and the bus address of `real` in that region.
///
/// A region with no real address holds no address.
fn region_at<'a>(
    tables: MmioTables<'a>,
    firmware: &'a str,
    real: u64,
) -> Option<(&'a MmioRegion, u64)> {
    regions_of(tables, firmware)
        .filter_map(Result::ok)
        .find_map(|(base, region)| {
            let offset = real.checked_sub(base).filter(|&off| off < region.size)?;
            Some((region, region.address + offset))
        })
}

/// The device access that the fault `fault` of hardware thread `thread`
/// in `result` gives, or `None` for any other fault.
///
/// The fault counts when all of these hold:
/// - it is the PPU's stop at unmodelled storage;
/// - the step's diagnostics hold the access's effective and real
///   addresses;
/// - an MMIO region of a device at `firmware` holds the real address.
pub(super) fn device_access(
    firmware: &str,
    thread: u8,
    fault: FaultKind,
    result: &ExecutionStepResult,
) -> Option<DeviceMmioAccess> {
    if fault != FaultKind::Guest(FAULT_UNMODELLED_STORAGE) {
        return None;
    }
    let diag = &result.local_diagnostics;
    let (ea, real) = (diag.faulting_ea?, diag.faulting_ra?);
    let (region, bus_address) = region_at(SHIPPED, firmware, real)?;
    Some(DeviceMmioAccess {
        thread,
        pc: diag.pc,
        region: *region,
        bus_address,
        real,
        ea,
    })
}
