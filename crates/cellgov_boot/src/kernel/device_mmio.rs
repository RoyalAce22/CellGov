//! The MMIO regions of the system bus devices in a kernel run's real
//! address space, and the stop of an access that reaches one.
//!
//! The console has storage at each region's real address: the device's
//! registers. CellGov does not model them, so the run declares each
//! region an unmodelled span. A span holds no bytes, so it never reads as
//! zeros, and a store there never lands. An access there stops the run
//! with the device that holds the address.
//!
//! A source gives some register words for the loads of one kernel
//! instruction each ([`DEVICE_REGISTER_READS`]). The run declares each
//! such word a sourced word of its span, so that instruction's load gets
//! the word, and every other access there still stops.

use cellgov_effects::FaultKind;
use cellgov_exec::ExecutionStepResult;
use cellgov_mem::{GuestMemory, MemError};
use cellgov_ppu::FAULT_UNMODELLED_STORAGE;
use cellgov_ps3_abi::hw::device_register::DEVICE_REGISTER_READS;
use cellgov_ps3_abi::lv1::device_bus::{MmioRegion, MMIO_REAL_BASE, MMIO_REGIONS};

#[cfg(test)]
#[path = "tests/device_mmio_register_tests.rs"]
mod register_tests;

/// An access of a hardware thread to the MMIO of a device.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeviceMmioAccess {
    /// The PPE hardware thread that made the access.
    pub thread: u8,
    /// The address of the instruction that made the access, when the
    /// unit named it.
    pub pc: Option<u64>,
    /// The MMIO region that holds the access: its bus, device, slot,
    /// bus address and size.
    pub region: MmioRegion,
    /// The bus address of the first byte that the access reaches.
    pub bus_address: u64,
    /// The real address of that byte.
    pub real: u64,
    /// The effective address of that byte.
    pub ea: u64,
}

/// The MMIO regions that lv1's init gives the devices at `firmware`,
/// each with its real address.
fn regions_of(firmware: &str) -> impl Iterator<Item = (u64, &'static MmioRegion)> + '_ {
    MMIO_REGIONS
        .iter()
        .filter(move |region| region.firmware == firmware)
        .map(|region| (MMIO_REAL_BASE + region.address, region))
}

/// Declares each MMIO region of the devices at `firmware` an unmodelled
/// span of `mem`, and each register word that a source gives at
/// `firmware` a sourced word of its span.
///
/// # Errors
///
/// Returns one of these:
/// - the refusal of [`GuestMemory::declare_unmodelled`] when a region
///   overlaps a region of `mem` or another span, or runs past the end of
///   the address space;
/// - the refusal of [`GuestMemory::declare_sourced_word`] when a word is
///   not an aligned word of one span, or repeats a word and reader. A
///   word of a region that lv1 does not give the device is in no span.
pub(super) fn declare_device_mmio(mem: &mut GuestMemory, firmware: &str) -> Result<(), MemError> {
    for (real, region) in regions_of(firmware) {
        mem.declare_unmodelled(real, region.size)?;
    }
    for read in DEVICE_REGISTER_READS
        .iter()
        .filter(|r| r.firmware == firmware)
    {
        let real = MMIO_REAL_BASE + read.region + read.offset;
        mem.declare_sourced_word(real, read.value, read.reader)?;
    }
    Ok(())
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
    let (_, region) = regions_of(firmware)
        .find(|&(base, region)| real.checked_sub(base).is_some_and(|off| off < region.size))?;
    Some(DeviceMmioAccess {
        thread,
        pc: diag.pc,
        region: *region,
        bus_address: real - MMIO_REAL_BASE,
        real,
        ea,
    })
}
