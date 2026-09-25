//! The address-space id, the space table, and the resolvers that pick a unit's or a space's memory.

use std::collections::BTreeMap;

use cellgov_event::UnitId;
use cellgov_mem::lanes::{self, source, LaneMap, LaneValue, ObjectLanes};
use cellgov_mem::{GuestMemory, MemError};
use cellgov_sync::ReservationTable;

/// Address-space id; space 0 is the boot process's space.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct AddressSpaceId(u32);

impl AddressSpaceId {
    /// The boot process's space, backed by `Runtime::memory`.
    pub const BOOT: AddressSpaceId = AddressSpaceId(0);

    /// Construct from a raw id.
    pub const fn new(raw: u32) -> Self {
        Self(raw)
    }

    /// Raw id value.
    pub fn raw(self) -> u32 {
        self.0
    }
}

/// Field 1 is the space id.
impl LaneValue for AddressSpaceId {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.0));
    }
}

/// One process-shared segment: a size and its per-space views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::runtime) struct SharedMapping {
    pub(super) size: u64,
    /// `(space, base)` in registration order; replication follows
    /// this order.
    pub(super) views: Vec<(AddressSpaceId, u64)>,
}

/// Lane fields of one mapping:
///
/// 1. `size`
/// 2. the view count
/// 3. slot `i`: the space of view `i`
/// 4. slot `i`: the base of view `i`
impl LaneValue for SharedMapping {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.size);
        lanes.lane(2, 0, self.views.len() as u64);
        for (i, (space, base)) in self.views.iter().enumerate() {
            lanes.lane(3, i as u64, u64::from(space.0));
            lanes.lane(4, i as u64, *base);
        }
    }
}

/// Child spaces, per-unit space tags, and shared mappings.
///
/// Empty tables contribute nothing to any hash channel, so
/// single-process boots hash identically whether or not the spaces
/// API is ever touched.
#[derive(Debug, Clone)]
pub(in crate::runtime) struct SpaceTable {
    /// Child spaces only; space 0 is `Runtime::memory`.
    pub(in crate::runtime) extra: BTreeMap<AddressSpaceId, GuestMemory>,
    /// Shared segments keyed by IPC key.
    pub(in crate::runtime) shared: LaneMap<u64, SharedMapping>,
    /// Unit -> space; absent means space 0.
    pub(in crate::runtime) unit_spaces: LaneMap<UnitId, AddressSpaceId>,
    /// Keyed-shm install history: IPC key -> (segment size, views in
    /// map order). A keyed map promotes into `shared` when a second
    /// space attaches. Outside the sync-state lanes, so a
    /// single-process boot's hash does not move.
    pub(in crate::runtime) keyed_installs: BTreeMap<u64, (u64, Vec<(AddressSpaceId, u64)>)>,
    /// Child-space reservation tables, keyed 1:1 with `extra`;
    /// space 0's table is `Runtime::reservations`.
    pub(in crate::runtime) extra_reservations: BTreeMap<AddressSpaceId, ReservationTable>,
}

/// Why a spaces-API call failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SpaceError {
    /// Space id already has a memory instance (or is space 0).
    #[error("address space 0x{0:08x} already exists")]
    SpaceExists(u32),
    /// Space id has no memory instance and is not space 0.
    #[error("address space 0x{0:08x} does not exist")]
    UnknownSpace(u32),
    /// IPC key already has a registered mapping.
    #[error("shared mapping key 0x{0:016x} already registered")]
    KeyExists(u64),
    /// Region installation into a view's space failed.
    #[error("shared view install failed: {0}")]
    ViewInstall(#[source] MemError),
}

impl Default for SpaceTable {
    fn default() -> Self {
        Self {
            extra: BTreeMap::new(),
            shared: LaneMap::new(source::SPACE_SHARED, |key| key),
            unit_spaces: LaneMap::new(source::SPACE_TAG, UnitId::raw),
            keyed_installs: BTreeMap::new(),
            extra_reservations: BTreeMap::new(),
        }
    }
}

impl SpaceTable {
    /// The space `unit` belongs to.
    pub(in crate::runtime) fn space_of(&self, unit: UnitId) -> AddressSpaceId {
        self.unit_spaces
            .get(unit)
            .copied()
            .unwrap_or(AddressSpaceId::BOOT)
    }

    /// The table's partial of the sync-state sum.
    ///
    /// The partial is the sum of:
    ///
    /// - a presence lane per child space, computed on read;
    /// - the partial of the unit tags;
    /// - the partial of the shared mappings.
    ///
    /// The committed-memory hash channel carries the content of the
    /// child spaces.
    pub(in crate::runtime) fn sync_partial(&self) -> u128 {
        self.spaces_term()
            .wrapping_add(self.unit_spaces.partial())
            .wrapping_add(self.shared.partial())
    }

    /// [`Self::sync_partial`] computed from every entry.
    pub(in crate::runtime) fn sync_partial_from_scratch(&self) -> u128 {
        self.spaces_term()
            .wrapping_add(self.unit_spaces.partial_from_scratch())
            .wrapping_add(self.shared.partial_from_scratch())
    }

    /// The presence lanes of the child spaces.
    fn spaces_term(&self) -> u128 {
        self.extra.keys().fold(0u128, |acc, space| {
            acc.wrapping_add(lanes::value_term(
                source::SPACE,
                u64::from(space.raw()),
                &(),
            ))
        })
    }
}

/// Resolve a unit's memory from the two backing fields directly, so
/// callers can hold other `Runtime` fields mutably at the same time.
pub(in crate::runtime) fn resolve_unit_memory<'a>(
    memory: &'a GuestMemory,
    spaces: &'a SpaceTable,
    unit: UnitId,
) -> &'a GuestMemory {
    match spaces.space_of(unit) {
        AddressSpaceId::BOOT => memory,
        s => spaces
            .extra
            .get(&s)
            .expect("unit tagged with a space that was never created"),
    }
}

/// Reservation-table twin of [`resolve_unit_memory`].
pub(in crate::runtime) fn resolve_unit_reservations<'a>(
    reservations: &'a ReservationTable,
    spaces: &'a SpaceTable,
    unit: UnitId,
) -> &'a ReservationTable {
    match spaces.space_of(unit) {
        AddressSpaceId::BOOT => reservations,
        s => spaces
            .extra_reservations
            .get(&s)
            .expect("unit tagged with a space that was never created"),
    }
}

/// Read view of `space`'s memory, resolved from the backing fields
/// directly (twin of [`resolve_space_memory_for_write`]).
pub(in crate::runtime) fn resolve_space_memory<'a>(
    memory: &'a GuestMemory,
    spaces: &'a SpaceTable,
    space: AddressSpaceId,
) -> &'a GuestMemory {
    match space {
        AddressSpaceId::BOOT => memory,
        s => spaces
            .extra
            .get(&s)
            .expect("read targeted a space that was never created"),
    }
}

/// Mutable view of `space`'s memory alone, for the regions a dispatch
/// installs -- a child thread's stack, an shm window. A write into
/// bytes a region already backs goes through [`Runtime::host_write`](crate::runtime::state::Runtime::host_write),
/// which resolves the space's reservation table with its memory.
pub(in crate::runtime) fn resolve_space_memory_for_write<'a>(
    memory: &'a mut GuestMemory,
    spaces: &'a mut SpaceTable,
    space: AddressSpaceId,
) -> &'a mut GuestMemory {
    match space {
        AddressSpaceId::BOOT => memory,
        s => spaces
            .extra
            .get_mut(&s)
            .expect("write targeted a space that was never created"),
    }
}

/// Mutable commit targets for `space`: its memory and its reservation
/// table, borrowed together so a caller holds both while other
/// `Runtime` fields stay free.
///
/// The third element is space 0's memory, `Some` only where `space` is
/// a child. The commit pipeline validates a DMA transfer's ends against
/// it, for the reason on [`crate::commit::CommitContext::dma_memory`].
pub(in crate::runtime) fn resolve_commit_targets<'a>(
    memory: &'a mut GuestMemory,
    reservations: &'a mut ReservationTable,
    spaces: &'a mut SpaceTable,
    space: AddressSpaceId,
) -> (
    &'a mut GuestMemory,
    &'a mut ReservationTable,
    Option<&'a GuestMemory>,
) {
    match space {
        AddressSpaceId::BOOT => (memory, reservations, None),
        s => (
            spaces.extra.get_mut(&s).unwrap_or_else(|| {
                panic!(
                    "commit targeted a space that was never created: 0x{:08x}",
                    s.raw()
                )
            }),
            spaces
                .extra_reservations
                .get_mut(&s)
                .expect("reservation table is created with its space"),
            Some(memory),
        ),
    }
}
