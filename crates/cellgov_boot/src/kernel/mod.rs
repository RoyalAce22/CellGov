//! The LV2 kernel of record: the stored kernel of the firmware a boot
//! selected, decrypted and mapped at its link address.
//!
//! The plaintext kernel exists only in guest memory. When a boot has
//! no kernel of record, [`kernel_of_record`] returns a
//! [`KernelUnavailable`] that says why, and the boot keeps the Rust LV2.

mod acquire;
mod device_mmio;
mod from_entry;
mod map;

pub use acquire::{kernel_of_record, KernelUnavailable, LoadedKernel};
pub use device_mmio::{DeviceMmioAccess, DeviceMmioError, MmioRealRefusal};
pub use from_entry::{
    kernel_partition, run_kernel_from_entry, KernelHypercall, KernelHypercallSite, KernelPosition,
    KernelRun, KernelRunError, KernelRunStop, RECENT_PCS,
};
pub use map::{KernelImage, KernelMapError};

#[cfg(test)]
#[path = "tests/map_tests.rs"]
mod map_tests;

#[cfg(test)]
#[path = "tests/acquire_tests.rs"]
mod acquire_tests;

#[cfg(test)]
#[path = "tests/from_entry_tests.rs"]
mod from_entry_tests;
