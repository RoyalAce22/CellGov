//! The facts of the device bus hypercalls: the devices, their MMIO and
//! DMA regions, the I/O segments and the IOMMU entries.
//!
//! The facts come from the 4.93 `lv1.self`, decrypted offline and read
//! with `cellgov dev disasm`. The repository holds no byte of lv1, only
//! these facts.

mod close;
mod dma_map;
mod dma_region;
mod game_os_mmio;
mod mmio_map;
mod mmio_region;
mod mmio_unmap;
mod open;
mod storage_bus;

pub use self::{
    close::*, dma_map::*, dma_region::*, game_os_mmio::*, mmio_map::*, mmio_region::*,
    mmio_unmap::*, open::*, storage_bus::*,
};

#[cfg(test)]
#[path = "tests/open_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/open_device_1_tests.rs"]
mod device_1_tests;

#[cfg(test)]
#[path = "tests/storage_bus_tests.rs"]
mod storage_bus_tests;

#[cfg(test)]
#[path = "tests/close_tests.rs"]
mod close_tests;

#[cfg(test)]
#[path = "tests/dma_region_tests.rs"]
mod dma_region_tests;

#[cfg(test)]
#[path = "tests/dma_region_console_tests.rs"]
mod dma_region_console_tests;

#[cfg(test)]
#[path = "tests/dma_slots_console_tests.rs"]
mod dma_slots_console_tests;

#[cfg(test)]
#[path = "tests/dma_slot_rows_console_tests.rs"]
mod dma_slot_rows_console_tests;

#[cfg(test)]
#[path = "tests/dma_device_2_console_tests.rs"]
mod dma_device_2_console_tests;

#[cfg(test)]
#[path = "tests/dma_segments_console_tests.rs"]
mod dma_segments_console_tests;

#[cfg(test)]
#[path = "tests/dma_map_tests.rs"]
mod dma_map_tests;

#[cfg(test)]
#[path = "tests/dma_map_console_tests.rs"]
mod dma_map_console_tests;

#[cfg(test)]
#[path = "tests/dma_mappings_console_tests.rs"]
mod dma_mappings_console_tests;

#[cfg(test)]
#[path = "tests/mmio_map_tests.rs"]
mod mmio_map_tests;

#[cfg(test)]
#[path = "tests/mmio_map_console_tests.rs"]
mod mmio_map_console_tests;

#[cfg(test)]
#[path = "tests/mmio_real_tests.rs"]
mod mmio_real_tests;

#[cfg(test)]
#[path = "tests/mmio_index_tests.rs"]
mod mmio_index_tests;

#[cfg(test)]
#[path = "tests/mmio_lists_console_tests.rs"]
mod mmio_lists_console_tests;

#[cfg(test)]
#[path = "tests/mmio_device_2_console_tests.rs"]
mod mmio_device_2_console_tests;

#[cfg(test)]
#[path = "tests/mmio_unmap_tests.rs"]
mod mmio_unmap_tests;
