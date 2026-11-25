//! The table of the hypercall groups, which the dispatch and the
//! answered numbers read.

use core::ops::RangeInclusive;

use cellgov_mem::scheme::HashedSurface;

use super::hypercall::Hypercall;
use super::{device_bus, gpu, interrupt, memory, network, partition, spe, storage, uart};

/// One hypercall group, as its own module declares it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Group {
    /// The group's name.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the registration guard reads the name")
    )]
    pub(crate) name: &'static str,
    /// The group's registrations: one list for each module that answers
    /// calls.
    pub(crate) hypercalls: &'static [&'static [Hypercall]],
    /// The lane source tags the group's state may take: a range of its
    /// own inside [`cellgov_mem::lanes::source::HYPERVISOR`].
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the lane source guard reads the ranges")
    )]
    pub(crate) lanes: &'static [RangeInclusive<u8>],
    /// The parts of the group's state that the sync-state hash covers,
    /// each with the checkpoint scheme version that first covers it.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the checkpoint scheme guard reads the surfaces")
    )]
    pub(crate) hashed_surfaces: &'static [HashedSurface],
}

impl Group {
    /// Each registration of the group, in declaration order.
    pub(crate) fn registrations(&self) -> impl Iterator<Item = &'static Hypercall> {
        self.hypercalls.iter().flat_map(|list| list.iter())
    }
}

/// Every hypercall group.
pub(crate) const GROUPS: &[Group] = &[
    memory::GROUP,
    interrupt::GROUP,
    partition::GROUP,
    device_bus::GROUP,
    spe::GROUP,
    storage::GROUP,
    gpu::GROUP,
    network::GROUP,
    uart::GROUP,
];

/// Each registration of each group.
pub(crate) fn registrations() -> impl Iterator<Item = &'static Hypercall> {
    GROUPS.iter().flat_map(Group::registrations)
}

/// The registration of hypercall `number`, or `None` when no group
/// answers it.
pub(crate) fn registration(number: u64) -> Option<&'static Hypercall> {
    registrations().find(|hypercall| hypercall.number == number)
}

#[cfg(test)]
#[path = "tests/registration_tests.rs"]
mod registration_tests;

#[cfg(test)]
#[path = "tests/scheme_tests.rs"]
mod scheme_tests;

#[cfg(test)]
#[path = "tests/lane_tests.rs"]
mod lane_tests;
