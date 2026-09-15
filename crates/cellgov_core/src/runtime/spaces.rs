//! Per-process address spaces and explicit process-shared mappings.
//!
//! Space 0 is the boot process's space and lives in `Runtime::memory`;
//! child spaces are further [`GuestMemory`] instances. Every unit
//! belongs to exactly one space (an untagged unit is in space 0), and
//! its execution context, syscall-parameter reads and commit batch all
//! resolve through that space.
//!
//! A shared segment is registered under an IPC key with one or more
//! `(space, base)` views; a space may map the segment at several bases.
//! Registration installs a zero-filled region in each view's space, and
//! the commit pipeline replicates a committed write that lands in one
//! view into every sibling view, same-space aliases included, inside
//! the same commit batch.
//!
//! Reservations are space-scoped: space 0's table is
//! `Runtime::reservations`, each child space owns its own
//! [`ReservationTable`], and the commit pipeline's clear sweep runs
//! against the emitting unit's table only. The one cross-space path is
//! a shared mapping: a replicated write and a promotion seed both clear
//! the reservations that cover the translated range in the sibling
//! view's space, each with the exemption its own writer carries (see
//! `Runtime::host_write`). A DMA transfer resolves both ends in
//! space 0, and a landing inside a shared view replicates through the
//! same path.

use std::collections::BTreeMap;

use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, MemError, PageSize};
use cellgov_sync::ReservationTable;
use cellgov_trace::HostWriter;

use super::state::Runtime;

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

/// One process-shared segment: a size and its per-space views.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SharedMapping {
    size: u64,
    /// `(space, base)` in registration order; replication follows
    /// this order.
    views: Vec<(AddressSpaceId, u64)>,
}

/// Child spaces, per-unit space tags, and shared mappings.
///
/// Empty tables contribute nothing to any hash channel, so
/// single-process boots hash identically whether or not the spaces
/// API is ever touched.
#[derive(Debug, Clone, Default)]
pub(super) struct SpaceTable {
    /// Child spaces only; space 0 is `Runtime::memory`.
    pub(super) extra: BTreeMap<AddressSpaceId, GuestMemory>,
    /// Shared segments keyed by IPC key.
    pub(super) shared: BTreeMap<u64, SharedMapping>,
    /// Unit -> space; absent means space 0.
    pub(super) unit_spaces: BTreeMap<UnitId, AddressSpaceId>,
    /// Keyed-shm install history: IPC key -> (segment size, views in
    /// map order). A keyed map promotes into `shared` when a second
    /// space attaches. Outside `metadata_hash` and `is_empty`, so a
    /// single-process boot's hash does not move.
    pub(super) keyed_installs: BTreeMap<u64, (u64, Vec<(AddressSpaceId, u64)>)>,
    /// Child-space reservation tables, keyed 1:1 with `extra`;
    /// space 0's table is `Runtime::reservations`.
    pub(super) extra_reservations: BTreeMap<AddressSpaceId, ReservationTable>,
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

impl SpaceTable {
    /// Whether any child space, tag, or mapping exists.
    pub(super) fn is_empty(&self) -> bool {
        self.extra.is_empty() && self.shared.is_empty() && self.unit_spaces.is_empty()
    }

    /// The space `unit` belongs to.
    pub(super) fn space_of(&self, unit: UnitId) -> AddressSpaceId {
        self.unit_spaces
            .get(&unit)
            .copied()
            .unwrap_or(AddressSpaceId::BOOT)
    }

    /// FNV-1a over tags and mapping metadata (content hashes of the
    /// child spaces travel on the committed-memory hash channel).
    pub(super) fn metadata_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&(self.unit_spaces.len() as u64).to_le_bytes());
        for (unit, space) in &self.unit_spaces {
            hasher.write(&unit.raw().to_le_bytes());
            hasher.write(&space.raw().to_le_bytes());
        }
        hasher.write(&(self.shared.len() as u64).to_le_bytes());
        for (key, mapping) in &self.shared {
            hasher.write(&key.to_le_bytes());
            hasher.write(&mapping.size.to_le_bytes());
            // The length prefix keeps a view entry distinct from the
            // next mapping's key and size in the hash stream.
            hasher.write(&(mapping.views.len() as u64).to_le_bytes());
            for (space, base) in &mapping.views {
                hasher.write(&space.raw().to_le_bytes());
                hasher.write(&base.to_le_bytes());
            }
        }
        hasher.finish()
    }
}

/// Resolve a unit's memory from the two backing fields directly, so
/// callers can hold other `Runtime` fields mutably at the same time.
pub(super) fn resolve_unit_memory<'a>(
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
pub(super) fn resolve_unit_reservations<'a>(
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
pub(super) fn resolve_space_memory<'a>(
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
/// bytes a region already backs goes through [`Runtime::host_write`],
/// which resolves the space's reservation table with its memory.
pub(super) fn resolve_space_memory_for_write<'a>(
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
pub(super) fn resolve_commit_targets<'a>(
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

impl Runtime {
    /// Create an empty child address space.
    ///
    /// [`Runtime::create_address_space_with`] takes the memory instead.
    ///
    /// # Errors
    /// [`SpaceError::SpaceExists`] for space 0 or a duplicate id.
    pub fn create_address_space(&mut self, space: AddressSpaceId) -> Result<(), SpaceError> {
        self.create_address_space_with(
            space,
            GuestMemory::from_regions(Vec::new()).expect("empty region set cannot overlap"),
        )
    }

    /// Create a child address space over `memory`.
    ///
    /// From here bytes reach the space through the commit pipeline or
    /// [`Runtime::place_bytes`].
    ///
    /// # Errors
    /// [`SpaceError::SpaceExists`] for space 0 or a duplicate id.
    pub fn create_address_space_with(
        &mut self,
        space: AddressSpaceId,
        memory: GuestMemory,
    ) -> Result<(), SpaceError> {
        if space == AddressSpaceId::BOOT || self.spaces.extra.contains_key(&space) {
            return Err(SpaceError::SpaceExists(space.raw()));
        }
        self.spaces.extra.insert(space, memory);
        self.spaces
            .extra_reservations
            .insert(space, ReservationTable::new());
        Ok(())
    }

    /// Every address space with its memory, space 0 first, then child
    /// spaces in id order.
    pub fn address_spaces(&self) -> impl Iterator<Item = (AddressSpaceId, &GuestMemory)> {
        std::iter::once((AddressSpaceId::BOOT, &self.memory))
            .chain(self.spaces.extra.iter().map(|(id, mem)| (*id, mem)))
    }

    /// Read view of `space`'s memory.
    ///
    /// # Errors
    /// [`SpaceError::UnknownSpace`] when no such space exists.
    pub fn space_memory(&self, space: AddressSpaceId) -> Result<&GuestMemory, SpaceError> {
        if space == AddressSpaceId::BOOT {
            return Ok(&self.memory);
        }
        self.spaces
            .extra
            .get(&space)
            .ok_or(SpaceError::UnknownSpace(space.raw()))
    }

    /// Mutable view of `space`'s memory, for a test that shapes a
    /// space in place.
    ///
    /// # Errors
    /// [`SpaceError::UnknownSpace`] when no such space exists.
    #[cfg(test)]
    pub(crate) fn space_memory_mut(
        &mut self,
        space: AddressSpaceId,
    ) -> Result<&mut GuestMemory, SpaceError> {
        if space == AddressSpaceId::BOOT {
            return Ok(&mut self.memory);
        }
        self.spaces
            .extra
            .get_mut(&space)
            .ok_or(SpaceError::UnknownSpace(space.raw()))
    }

    /// Read view of `space`'s reservation table.
    ///
    /// # Errors
    /// [`SpaceError::UnknownSpace`] when no such space exists.
    pub fn space_reservations(
        &self,
        space: AddressSpaceId,
    ) -> Result<&ReservationTable, SpaceError> {
        if space == AddressSpaceId::BOOT {
            return Ok(&self.reservations);
        }
        self.spaces
            .extra_reservations
            .get(&space)
            .ok_or(SpaceError::UnknownSpace(space.raw()))
    }

    /// Mutable view of `space`'s reservation table, for a test that
    /// seeds a reservation.
    ///
    /// # Errors
    /// [`SpaceError::UnknownSpace`] when no such space exists.
    #[cfg(test)]
    pub(crate) fn space_reservations_mut(
        &mut self,
        space: AddressSpaceId,
    ) -> Result<&mut ReservationTable, SpaceError> {
        if space == AddressSpaceId::BOOT {
            return Ok(&mut self.reservations);
        }
        self.spaces
            .extra_reservations
            .get_mut(&space)
            .ok_or(SpaceError::UnknownSpace(space.raw()))
    }

    /// Assign `unit` to `space`. Untagged units are space 0; tagging
    /// back to [`AddressSpaceId::BOOT`] removes the entry.
    ///
    /// # Errors
    /// [`SpaceError::UnknownSpace`] when the space does not exist.
    pub fn assign_unit_space(
        &mut self,
        unit: UnitId,
        space: AddressSpaceId,
    ) -> Result<(), SpaceError> {
        if space == AddressSpaceId::BOOT {
            self.spaces.unit_spaces.remove(&unit);
            return Ok(());
        }
        if !self.spaces.extra.contains_key(&space) {
            return Err(SpaceError::UnknownSpace(space.raw()));
        }
        self.spaces.unit_spaces.insert(unit, space);
        Ok(())
    }

    /// The space `unit` executes in.
    pub fn unit_space(&self, unit: UnitId) -> AddressSpaceId {
        self.spaces.space_of(unit)
    }

    /// Register a process-shared segment under `key` with one
    /// `(space, base)` view per participating space, installing a
    /// zero-filled region in each. Views stay coherent from here on:
    /// commits landing in one view replicate to the others.
    ///
    /// # Errors
    /// - [`SpaceError::KeyExists`] on a duplicate key.
    /// - [`SpaceError::UnknownSpace`] when a view names a missing space.
    /// - [`SpaceError::ViewInstall`] when a view's range overlaps an
    ///   existing region in its space, another view in this call, or
    ///   the end of the address space; nothing is installed and the
    ///   mapping is not registered.
    pub fn register_shared_mapping(
        &mut self,
        key: u64,
        size: u64,
        views: &[(AddressSpaceId, u64)],
    ) -> Result<(), SpaceError> {
        if self.spaces.shared.contains_key(&key) {
            return Err(SpaceError::KeyExists(key));
        }
        for &(space, _) in views {
            if space != AddressSpaceId::BOOT && !self.spaces.extra.contains_key(&space) {
                return Err(SpaceError::UnknownSpace(space.raw()));
            }
        }
        // Every view is checked before any is installed: there is no
        // region-removal API, and an orphaned region would shift
        // `committed_memory_hash`.
        for (idx, &(space, base)) in views.iter().enumerate() {
            let end = u128::from(base) + u128::from(size);
            if end > u128::from(u64::MAX) {
                return Err(SpaceError::ViewInstall(MemError::OverlappingRegions));
            }
            let mem = match space {
                AddressSpaceId::BOOT => &self.memory,
                s => self.spaces.extra.get(&s).expect("presence checked above"),
            };
            // `install_region`'s own rejection predicate, a zero-size
            // view at an existing base included.
            let rejects = |other_base: u128, other_end: u128| {
                (other_base <= u128::from(base) && other_end > u128::from(base))
                    || (other_base > u128::from(base) && end > other_base)
            };
            let overlaps_existing = mem.regions().any(|r| {
                rejects(
                    u128::from(r.base()),
                    u128::from(r.base()) + u128::from(r.size()),
                )
            });
            let overlaps_earlier_view = views[..idx].iter().any(|&(other_space, other_base)| {
                other_space == space
                    && rejects(
                        u128::from(other_base),
                        u128::from(other_base) + u128::from(size),
                    )
            });
            if overlaps_existing || overlaps_earlier_view {
                return Err(SpaceError::ViewInstall(MemError::OverlappingRegions));
            }
        }
        for &(space, base) in views {
            let mem = match space {
                AddressSpaceId::BOOT => &mut self.memory,
                s => self
                    .spaces
                    .extra
                    .get_mut(&s)
                    .expect("presence checked above"),
            };
            mem.install_region(base, size as usize, "shared", PageSize::Page64K)
                .map_err(SpaceError::ViewInstall)?;
        }
        self.spaces.shared.insert(
            key,
            SharedMapping {
                size,
                views: views.to_vec(),
            },
        );
        Ok(())
    }

    /// Record a keyed-shm window install (the 334 / 337 drain) and keep
    /// the views of one segment coherent across address spaces.
    ///
    /// A mapping registers in [`SpaceTable::shared`] when a second
    /// space attaches. Promotion adopts every recorded view and seeds
    /// each from the first, because an attach observes the content
    /// written before it and a repeat map inside the first space holds
    /// its own zero-filled region. A later attach appends to the live
    /// mapping the same way.
    pub(super) fn attach_keyed_shm_view(
        &mut self,
        key: u64,
        size: u64,
        space: AddressSpaceId,
        base: u64,
    ) {
        // A view of another length is not a view of this segment;
        // appending it would break every later containment test.
        if let Some(mapping) = self.spaces.shared.get(&key) {
            if mapping.size != size {
                self.lv2_host.log_invariant_break(
                    "spaces.keyed_shm_size_drift",
                    format_args!(
                        "keyed shm 0x{key:016x} mapped with size 0x{size:x} against a live \
                         mapping of size 0x{0:x}; view at 0x{base:x} left unreplicated",
                        mapping.size,
                    ),
                );
                return;
            }
        }
        let entry = self
            .spaces
            .keyed_installs
            .entry(key)
            .or_insert((size, Vec::new()));
        if entry.0 != size {
            self.lv2_host.log_invariant_break(
                "spaces.keyed_shm_size_drift",
                format_args!(
                    "keyed shm 0x{key:016x} mapped with size 0x{size:x} after size \
                     0x{0:x}; view at 0x{base:x} left unreplicated",
                    entry.0,
                ),
            );
            return;
        }
        entry.1.push((space, base));
        if self.spaces.shared.contains_key(&key) {
            self.adopt_shared_view(key, size, space, base);
            return;
        }
        let distinct_spaces: std::collections::BTreeSet<AddressSpaceId> =
            entry.1.iter().map(|&(s, _)| s).collect();
        if distinct_spaces.len() < 2 {
            return;
        }
        let views = self
            .spaces
            .keyed_installs
            .get(&key)
            .expect("entry inserted above")
            .1
            .clone();
        // Validate every view before mutating anything: each region
        // was installed by its own drain, so a miss here means the
        // install stream and this bookkeeping diverged.
        for &(view_space, view_base) in &views {
            if !self.shared_view_backed(view_space, view_base, size) {
                self.lv2_host.log_invariant_break(
                    "spaces.keyed_shm_view_unbacked",
                    format_args!(
                        "keyed shm 0x{key:016x}: view at 0x{view_base:x}+0x{size:x} in \
                         space {} has no backing region; replication not registered",
                        view_space.raw(),
                    ),
                );
                return;
            }
        }
        // Every view, the earlier ones included, seeds from the first;
        // see the doc on `attach_keyed_shm_view`.
        let (first, rest) = views.split_first().expect("promotion needs two views");
        for &view in rest {
            self.copy_shared_segment(*first, view, size);
        }
        self.spaces
            .shared
            .insert(key, SharedMapping { size, views });
    }

    /// Append one view to a live keyed mapping, seeding it from the
    /// mapping's first view.
    fn adopt_shared_view(&mut self, key: u64, size: u64, space: AddressSpaceId, base: u64) {
        if !self.shared_view_backed(space, base, size) {
            self.lv2_host.log_invariant_break(
                "spaces.keyed_shm_view_unbacked",
                format_args!(
                    "keyed shm 0x{key:016x}: attaching view at 0x{base:x}+0x{size:x} in \
                     space {} has no backing region; view not added",
                    space.raw(),
                ),
            );
            return;
        }
        let first = self.spaces.shared[&key].views[0];
        self.copy_shared_segment(first, (space, base), size);
        self.spaces
            .shared
            .get_mut(&key)
            .expect("caller checked the key is live")
            .views
            .push((space, base));
    }

    /// Whether `space` has one `ReadWrite` region wholly containing
    /// `[base, base+size)`. Read-only or reserved backing does not
    /// count: [`Runtime::copy_shared_segment`] reads and writes the
    /// whole window, and a non-`ReadWrite` region would fail both.
    fn shared_view_backed(&self, space: AddressSpaceId, base: u64, size: u64) -> bool {
        let mem = match space {
            AddressSpaceId::BOOT => &self.memory,
            s => match self.spaces.extra.get(&s) {
                Some(m) => m,
                None => return false,
            },
        };
        mem.containing_region(base, size)
            .is_some_and(|r| r.access() == cellgov_mem::RegionAccess::ReadWrite)
    }

    /// Copy the segment bytes visible through `src` into `dst`, and
    /// drop the reservations `dst`'s space holds over the rewritten
    /// bytes. No-op for the trivial self-copy.
    fn copy_shared_segment(
        &mut self,
        src: (AddressSpaceId, u64),
        dst: (AddressSpaceId, u64),
        size: u64,
    ) {
        if src == dst {
            return;
        }
        let src_range = ByteRange::new(GuestAddr::new(src.1), size).expect("validated view range");
        let bytes: Vec<u8> = {
            let mem = match src.0 {
                AddressSpaceId::BOOT => &self.memory,
                s => self.spaces.extra.get(&s).expect("validated view space"),
            };
            mem.read(src_range)
                .expect("validated backing region is readable")
                .to_vec()
        };
        let dst_range = ByteRange::new(GuestAddr::new(dst.1), size).expect("validated view range");
        // A seed is the host's copy, not a unit's store, so it exempts
        // nobody; see the doc on `Runtime::host_write`.
        // [PPC-Book2 p:10 s:1.7.3.1] a modification by some other
        // mechanism loses the reservation.
        self.host_write(HostWriter::SharedViewSeed, dst.0, dst_range, &bytes, None)
            .expect("validated backing region accepts the segment write");
    }

    /// Replicate the committed writes that landed in a shared view into
    /// every sibling view, and return the reservations that cleared.
    ///
    /// Runs after a successful commit, inside the same batch boundary;
    /// mappings in key order, views in registration order. Each write
    /// spares its emitter's reservation; see
    /// [`Runtime::fanout_committed_range`].
    pub(super) fn fanout_shared_writes(
        &mut self,
        source_space: AddressSpaceId,
        effects: &[cellgov_effects::Effect],
    ) -> usize {
        if self.spaces.shared.is_empty() {
            return 0;
        }
        let mut cleared = 0usize;
        for effect in effects {
            let (range, source) = match effect {
                cellgov_effects::Effect::SharedWriteIntent { range, source, .. } => {
                    (*range, *source)
                }
                // A conditional store commits to the source space only,
                // so one through a shared view leaves the siblings
                // incoherent. No producer does that yet; the assertion
                // names the first.
                cellgov_effects::Effect::ConditionalStore { range, .. } => {
                    debug_assert!(
                        !self.range_intersects_shared_view(source_space, *range),
                        "ConditionalStore at {:#x}+{:#x} targets a shared view; \
                         cross-space atomic replication is not modeled",
                        range.start().raw(),
                        range.length(),
                    );
                    // The assertion compiles out under `--release`; the
                    // invariant break covers that profile.
                    if self.range_intersects_shared_view(source_space, *range) {
                        self.lv2_host.log_invariant_break(
                            "spaces.conditional_store_targets_shared_view",
                            format_args!(
                                "ConditionalStore at 0x{:x}+0x{:x} targets a shared view \
                                 in space {}; cross-space atomic replication is not \
                                 modeled, sibling views are now incoherent",
                                range.start().raw(),
                                range.length(),
                                source_space.raw(),
                            ),
                        );
                    }
                    continue;
                }
                _ => continue,
            };
            // The storing unit keeps its reservation over every alias;
            // see the doc on `Runtime::fanout_committed_range`.
            // [PPC-Book2 p:10 s:1.7.3.1] the granule holds the real
            // address an effective address maps to, and only another
            // processor's store clears it.
            cleared += self.fanout_committed_range(source_space, range, Some(source));
        }
        cleared
    }

    /// Replicate one committed range from `source_space` into every
    /// sibling view of the shared segment it lands in, and return the
    /// reservations that cleared.
    ///
    /// `exempt` is the one unit whose reservation survives the
    /// replicated write: [`Runtime::fanout_shared_writes`] passes the
    /// store's emitter, a DMA landing its issuer. A unit belongs to one
    /// space, so the id matches a holder in one table and nothing in
    /// every other.
    /// [PPC-Book2 p:10 s:1.7.3.1] a reservation granule holds the real
    /// address an effective address maps to, so every view of one
    /// segment names one granule, and only another processor's store
    /// clears it.
    ///
    /// The bytes are read back out of committed memory, so a
    /// partial-overlap write replicates what the pipeline applied.
    pub(super) fn fanout_committed_range(
        &mut self,
        source_space: AddressSpaceId,
        range: ByteRange,
        exempt: Option<UnitId>,
    ) -> usize {
        if self.spaces.shared.is_empty() {
            return 0;
        }
        let mut cleared = 0usize;
        let (start, len) = (range.start().raw(), range.length());
        let mut replications: Vec<(Vec<u8>, AddressSpaceId, u64)> = Vec::new();
        for mapping in self.spaces.shared.values() {
            let source_view = mapping
                .views
                .iter()
                .find(|(space, base)| {
                    *space == source_space && start >= *base && start + len <= *base + mapping.size
                })
                .copied();
            let Some((_, source_base)) = source_view else {
                continue;
            };
            let offset = start - source_base;
            let committed: Vec<u8> = {
                let mem = match source_space {
                    AddressSpaceId::BOOT => &self.memory,
                    s => self.spaces.extra.get(&s).expect("source space exists"),
                };
                match mem.read(range) {
                    Some(bytes) => bytes.to_vec(),
                    // An unreadable source range committed nothing, so
                    // there is nothing to replicate; only an effect that
                    // never landed reaches here.
                    None => continue,
                }
            };
            for &(dst_space, dst_base) in &mapping.views {
                // Skip only the view the write landed in; a second view
                // in the same space aliases the same bytes.
                if dst_space == source_space && dst_base == source_base {
                    continue;
                }
                replications.push((committed.clone(), dst_space, dst_base + offset));
            }
        }
        for (bytes, dst_space, dst_addr) in replications {
            let len = bytes.len() as u64;
            let dst_range = ByteRange::new(GuestAddr::new(dst_addr), len)
                .expect("replication range mirrors a validated committed range");
            cleared += self
                .host_write(
                    HostWriter::SharedViewFanout,
                    dst_space,
                    dst_range,
                    &bytes,
                    exempt,
                )
                .expect("sibling view region installed at registration");
        }
        cleared
    }

    /// Sibling-view aliases of `range` as seen from `unit`'s space;
    /// [`Runtime::shared_alias_ranges_in`] with the unit's space.
    pub fn shared_alias_ranges(&self, unit: UnitId, range: ByteRange) -> Vec<ByteRange> {
        self.shared_alias_ranges_in(self.spaces.space_of(unit), range)
    }

    /// Sibling-view aliases of `range` as seen from `space`: for every
    /// shared mapping whose view in `space` wholly contains `range`,
    /// the equivalent range through each other view. A range that
    /// straddles a view's end gives nothing. The commit pipeline's
    /// fanout replicates under the same containment, so the aliases
    /// are the bytes a commit would reach.
    ///
    /// Dependency analysis reads these so two cross-space writes to the
    /// same shared bytes never prove independent. A host write names
    /// its own space, which [`Runtime::last_host_writes`] carries
    /// beside each range.
    pub fn shared_alias_ranges_in(
        &self,
        space: AddressSpaceId,
        range: ByteRange,
    ) -> Vec<ByteRange> {
        let (start, len) = (range.start().raw(), range.length());
        let mut out = Vec::new();
        for mapping in self.spaces.shared.values() {
            let source_view = mapping
                .views
                .iter()
                .find(|(view_space, base)| {
                    *view_space == space && start >= *base && start + len <= *base + mapping.size
                })
                .copied();
            let Some((_, source_base)) = source_view else {
                continue;
            };
            let offset = start - source_base;
            for &(other_space, other_base) in &mapping.views {
                // Skip only the view `range` itself lies in; a second
                // view in the same space aliases the same bytes.
                if other_space == space && other_base == source_base {
                    continue;
                }
                if let Some(alias) = ByteRange::new(GuestAddr::new(other_base + offset), len) {
                    out.push(alias);
                }
            }
        }
        out
    }

    /// Whether `range` lies inside any shared view of `space`.
    pub(super) fn range_intersects_shared_view(
        &self,
        space: AddressSpaceId,
        range: cellgov_mem::ByteRange,
    ) -> bool {
        let (start, len) = (range.start().raw(), range.length());
        self.spaces.shared.values().any(|mapping| {
            mapping.views.iter().any(|&(view_space, base)| {
                view_space == space
                    && start < base + mapping.size
                    && start.saturating_add(len) > base
            })
        })
    }

    /// Committed-memory hash over every space's content.
    ///
    /// Child spaces fold in so a cross-process divergence in one is
    /// witnessed; with no child space this is space 0's content hash.
    /// Mapping metadata stays outside it and reaches the sync-channel
    /// state hash through `metadata_hash`.
    pub fn committed_memory_hash(&self) -> u64 {
        if self.spaces.extra.is_empty() {
            return self.memory.content_hash();
        }
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&self.memory.content_hash().to_le_bytes());
        for (space, mem) in &self.spaces.extra {
            hasher.write(&space.raw().to_le_bytes());
            hasher.write(&mem.content_hash().to_le_bytes());
        }
        hasher.finish()
    }

    /// The schedule explorer's observable: [`Runtime::committed_memory_hash`]
    /// folded with every unit's private memory, in unit-id order.
    ///
    /// A unit that reports no private memory
    /// ([`cellgov_exec::ExecutionUnit::local_memory_hash`]) contributes
    /// nothing. A runtime whose units all report none hashes exactly as
    /// `committed_memory_hash` does. Each contributing unit folds its id
    /// beside its hash, so two units with exchanged local stores read as
    /// a different state.
    pub fn observable_hash(&self) -> u64 {
        let committed = self.committed_memory_hash();
        let mut contributors = self
            .registry
            .iter()
            .filter_map(|(id, unit)| unit.local_memory_hash().map(|hash| (id, hash)))
            .peekable();
        if contributors.peek().is_none() {
            return committed;
        }
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&committed.to_le_bytes());
        for (id, hash) in contributors {
            hasher.write(&id.raw().to_le_bytes());
            hasher.write(&hash.to_le_bytes());
        }
        hasher.finish()
    }
}

#[cfg(test)]
#[path = "tests/observable_hash_tests.rs"]
mod observable_hash_tests;

#[cfg(test)]
#[path = "tests/spaces_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/shared_view_store_tests.rs"]
mod shared_view_store_tests;
