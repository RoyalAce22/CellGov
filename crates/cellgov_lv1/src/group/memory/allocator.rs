//! The partition's region allocator: the address and the counter value
//! that lv1's region function gives a new region.
//!
//! lv1 inserts each region address in one set for the partition, and an
//! address that the set holds is taken; see
//! [`cellgov_ps3_abi::lv1::memory`]. These regions take their address
//! from the counter, and CellGov holds each of them:
//! - an allocated memory region;
//! - a page table mapping;
//! - a logical SPE's area and shadow page;
//! - a device MMIO mapping.
//!
//! The set also holds these regions, which CellGov does not hold:
//! - The region at address 0 that the function's fixed path inserts
//!   [OFW-4.93 lv1 a:0x2CEC88-0x2CEC94] [OFW-4.93 lv1 a:0x2CED2C-0x2CED30]
//!   [OFW-4.93 lv1 a:0x2CEE3C-0x2CEE84]. A counter address holds an
//!   exponent of 12 or more in bits 42 to 47, so it never equals 0.
//! - The regions that lv1 made before the kernel's first allocation,
//!   with the counter values below the boot value. No source gives their
//!   sizes. So CellGov refuses a counter value whose address field holds
//!   one of those values.

use std::collections::BTreeSet;

use cellgov_ps3_abi::lv1::memory::{region_address, region_counter_values};

use crate::group::state::GroupStates;

/// The logical partition addresses of the counter regions that CellGov
/// holds for the partition.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct HeldRegions(BTreeSet<u64>);

impl HeldRegions {
    /// The addresses of each region kind in `groups` that lv1's set holds.
    pub(crate) fn of(groups: &GroupStates) -> Self {
        let memory = groups.memory();
        let allocated = memory.regions().regions().iter().map(|r| r.lpar_address);
        let tables = memory.spaces().spaces().iter().filter_map(|s| s.mapping);
        let spes = groups
            .spe()
            .spes()
            .spes()
            .values()
            .flat_map(|spe| [spe.area, spe.shadow]);
        let mmio = groups.device_bus().mmio_mappings().map(|m| m.lpar_address);
        Self(allocated.chain(tables).chain(spes).chain(mmio).collect())
    }

    fn holds(&self, address: u64) -> bool {
        self.0.contains(&address)
    }

    fn len(&self) -> u64 {
        self.0.len() as u64
    }
}

/// The address that the region function gives and the counter value that
/// the region took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Placement {
    /// The region's logical partition address.
    pub(crate) address: u64,
    /// The counter value of the last attempt. The counter's next value is
    /// one more.
    pub(crate) counter: u64,
}

/// Why CellGov gives no address for a region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NoPlacement {
    /// The size is above `REGION_SIZE_MAX`. lv1 fails the call, and no
    /// image read gives the status of the hypercall.
    TooLarge,
    /// An attempt's address field holds a counter value below the boot
    /// value, and a region of an unknown size may hold that address.
    BootRegion,
    /// Each address of the size is taken. lv1 fails the call, and no
    /// image read gives the status of the hypercall.
    EveryAddressTaken,
}

impl NoPlacement {
    /// The refusal text.
    pub(crate) const fn what(self) -> &'static str {
        match self {
            Self::TooLarge => {
                "a region larger than 2^42 bytes, for which no read gives lv1's hypercall status"
            }
            Self::BootRegion => {
                "a region whose wrapped counter value a region that lv1 made before the kernel's \
                 first allocation may hold"
            }
            Self::EveryAddressTaken => {
                "a region whose every address the partition's regions hold, for which no read \
                 gives lv1's hypercall status"
            }
        }
    }
}

/// The address and counter value that lv1's region function gives a
/// region of `size` bytes, from the counter value `first`.
///
/// `boot` is the counter value at the kernel's first allocation. Each
/// attempt takes the next counter value, and an address in `held` is
/// taken. The loop makes at most `held.len() + 1` attempts:
/// - Attempts below the count of counter values give distinct
///   addresses, and `held` holds at most `held.len()` of them.
/// - When that count is `held.len()` or less, the loop makes as many
///   attempts as lv1, and `held` can hold every address.
///
/// # Errors
///
/// Returns the [`NoPlacement`] reason. With a `boot` above 0, a run
/// through every counter value meets a value below `boot` first. So only
/// a `boot` of 0 gives [`NoPlacement::EveryAddressTaken`].
pub(crate) fn place(
    size: u64,
    first: u64,
    boot: u64,
    held: &HeldRegions,
) -> Result<Placement, NoPlacement> {
    let Some(values) = region_counter_values(size) else {
        return Err(NoPlacement::TooLarge);
    };
    for attempt in 0..values.min(held.len().saturating_add(1)) {
        let counter = first.wrapping_add(attempt);
        if counter & (values - 1) < boot {
            return Err(NoPlacement::BootRegion);
        }
        let Some(address) = region_address(size, counter) else {
            return Err(NoPlacement::TooLarge);
        };
        if !held.holds(address) {
            return Ok(Placement { address, counter });
        }
    }
    Err(NoPlacement::EveryAddressTaken)
}

#[cfg(test)]
#[path = "tests/allocator_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/allocator_handler_tests.rs"]
mod handler_tests;
