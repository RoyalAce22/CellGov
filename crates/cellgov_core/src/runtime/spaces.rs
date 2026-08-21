//! Per-process address spaces and explicit process-shared mappings.
//!
//! Space 0 is the boot process's space and lives in `Runtime::memory`;
//! child spaces are additional [`GuestMemory`] instances. Every unit
//! belongs to exactly one space (untagged units are space 0), and a
//! step's execution context, syscall-parameter reads, and commit batch
//! all resolve through the unit's space.
//!
//! Sharing model: a shared segment is registered under an IPC key with
//! one or more `(space, base)` views; a space may map the segment at
//! several bases (the kernel admits repeated maps of one shared
//! segment -- RPCS3 `sys_mmapper.cpp` `sys_mmapper_map_shared_memory`
//! counts maps rather than rejecting a second one). Registration
//! installs a zero-filled region in each view's space; the commit
//! pipeline then keeps the views coherent by replicating committed
//! writes that land in one view into every sibling view -- same-space
//! aliases included -- within the same commit batch (views iterate in
//! registration order, so the replication order is deterministic).
//!
//! Reservations are space-scoped: space 0's table is
//! `Runtime::reservations`, each child space owns its own
//! [`ReservationTable`], and the commit pipeline's clear-sweeps run
//! against the emitting unit's table only -- equal numeric addresses
//! in different spaces are different memory and never alias. The one
//! cross-space path is a shared mapping: both replicating a committed
//! write into a sibling view and seeding a view when the segment
//! promotes clear reservations covering the translated range in that
//! view's space. DMA stays space 0 end to
//! end (payloads land in `Runtime::memory`, the completion sweep
//! hits the space-0 table).

use std::collections::BTreeMap;

use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, MemError, PageSize};
use cellgov_sync::ReservationTable;

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
    /// Shared segments keyed by IPC key (mmapper substrate).
    pub(super) shared: BTreeMap<u64, SharedMapping>,
    /// Unit -> space; absent means space 0.
    pub(super) unit_spaces: BTreeMap<UnitId, AddressSpaceId>,
    /// Keyed-shm install history: ipc key -> (segment size, views in
    /// map order). Pre-registration bookkeeping only -- a keyed map
    /// promotes into `shared` the moment a second address space
    /// attaches. Excluded from `metadata_hash` and `is_empty`:
    /// single-space entries are derivable from the install stream and
    /// must not perturb single-process boot hashes.
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
            // Length-prefix the views vec like the two maps above, so
            // a view entry cannot be confused with the next mapping's
            // key/size bytes in the hash stream.
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

/// Mutable view of `space`'s memory alone, for LV2-side direct
/// writes (e.g. the spawn pid writeback into the caller's space).
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
/// table, borrowed together so the commit context can hold both while
/// other `Runtime` fields stay free.
pub(super) fn resolve_commit_targets<'a>(
    memory: &'a mut GuestMemory,
    reservations: &'a mut ReservationTable,
    spaces: &'a mut SpaceTable,
    space: AddressSpaceId,
) -> (&'a mut GuestMemory, &'a mut ReservationTable) {
    match space {
        AddressSpaceId::BOOT => (memory, reservations),
        s => (
            spaces
                .extra
                .get_mut(&s)
                .expect("commit targeted a space that was never created"),
            spaces
                .extra_reservations
                .get_mut(&s)
                .expect("reservation table is created with its space"),
        ),
    }
}

impl Runtime {
    /// Create an empty child address space. The caller installs
    /// regions via [`Runtime::space_memory_mut`].
    ///
    /// # Errors
    /// [`SpaceError::SpaceExists`] for space 0 or a duplicate id.
    pub fn create_address_space(&mut self, space: AddressSpaceId) -> Result<(), SpaceError> {
        if space == AddressSpaceId::BOOT || self.spaces.extra.contains_key(&space) {
            return Err(SpaceError::SpaceExists(space.raw()));
        }
        self.spaces.extra.insert(
            space,
            GuestMemory::from_regions(Vec::new()).expect("empty region set cannot overlap"),
        );
        self.spaces
            .extra_reservations
            .insert(space, ReservationTable::new());
        Ok(())
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

    /// Mutable view of `space`'s memory, for boot-time region installs
    /// and image loads. Committed-state changes mid-run belong to the
    /// commit pipeline, exactly as with [`Runtime::memory_mut`].
    ///
    /// # Errors
    /// [`SpaceError::UnknownSpace`] when no such space exists.
    pub fn space_memory_mut(
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

    /// Mutable view of `space`'s reservation table, for test seeding;
    /// mid-run mutation belongs to the commit pipeline, exactly as
    /// with [`Runtime::reservations_mut`].
    ///
    /// # Errors
    /// [`SpaceError::UnknownSpace`] when no such space exists.
    pub fn space_reservations_mut(
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
        // All-or-nothing: check every view before installing any, so a
        // rejected view leaves no orphaned "shared" regions behind
        // (there is no region-removal API to roll a partial install
        // back with, and an orphan would shift committed_memory_hash
        // and block the address range forever).
        for (idx, &(space, base)) in views.iter().enumerate() {
            let end = u128::from(base) + u128::from(size);
            if end > u128::from(u64::MAX) {
                return Err(SpaceError::ViewInstall(MemError::OverlappingRegions));
            }
            let mem = match space {
                AddressSpaceId::BOOT => &self.memory,
                s => self.spaces.extra.get(&s).expect("presence checked above"),
            };
            // Exactly `install_region`'s rejection predicate (a
            // zero-size view at an existing base is rejected there
            // too).
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

    /// Record a keyed-shm window install (334 / 337 drain) and keep
    /// views of one segment coherent across address spaces.
    ///
    /// Single-space maps only book-keep: the mapping registers in
    /// [`SpaceTable::shared`] the moment a second address space
    /// attaches, adopting every recorded view (their regions are
    /// already installed by the drain) and seeding each of them from
    /// the first view -- an attach must observe content written
    /// before it, and a repeat map inside the first space got its own
    /// zero-filled region rather than the segment's bytes. Later
    /// attaches append to the live mapping the same way.
    /// Single-process boots therefore never touch the shared table.
    pub(super) fn attach_keyed_shm_view(
        &mut self,
        key: u64,
        size: u64,
        space: AddressSpaceId,
        base: u64,
    ) {
        // A live mapping's size is the segment's size; a view of a
        // different length is not a view of this segment, and
        // appending it would corrupt every later containment test.
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
        // Every view is one map of the same segment -- the kernel
        // installs one backing store at each mapped address (RPCS3
        // sys_mmapper.cpp sys_mmapper_map_shared_memory maps the
        // handle's shm object into every window it claims). Until
        // promotion each view was an independent zero-filled region,
        // so bring them ALL up to the first view's content, not just
        // the one attaching now: a repeat map inside the first space
        // would otherwise stay silently stale forever.
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
        let (mem, dst_reservations) = match dst.0 {
            AddressSpaceId::BOOT => (&mut self.memory, &mut self.reservations),
            s => (
                self.spaces.extra.get_mut(&s).expect("validated view space"),
                self.spaces
                    .extra_reservations
                    .get_mut(&s)
                    .expect("reservation table is created with its space"),
            ),
        };
        mem.apply_commit(dst_range, &bytes)
            .expect("validated backing region accepts the segment write");
        // Seeding rewrites the destination view's bytes, so it
        // invalidates every reservation covering them -- the same rule
        // `fanout_shared_writes` applies to a replicated store, and
        // the module's cross-space reservation contract.
        dst_reservations.clear_covering(dst.1, size, None);
    }

    /// Replicate committed writes that landed in a shared view into
    /// every sibling view. Runs after a successful commit, inside the
    /// same batch boundary; iterates mappings in key order and views
    /// in registration order. Each replicated write also clears
    /// reservations covering the translated range in the sibling
    /// view's space (a store through one view is a store to the
    /// shared bytes every view names); returns the count cleared.
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
            let range = match effect {
                cellgov_effects::Effect::SharedWriteIntent { range, .. } => *range,
                // Conditional stores commit to the source space only;
                // an atomic op through a shared view would leave
                // sibling views incoherent. No producer targets
                // shared segments with atomics yet -- surface the
                // first one here instead of as silent incoherence.
                cellgov_effects::Effect::ConditionalStore { range, .. } => {
                    debug_assert!(
                        !self.range_intersects_shared_view(source_space, *range),
                        "ConditionalStore at {:#x}+{:#x} targets a shared view; \
                         cross-space atomic replication is not modeled",
                        range.start().raw(),
                        range.length(),
                    );
                    // Release builds compile the assert out; keep the
                    // witness loud there too (same guard pattern as
                    // dispatch.lv2_write_targets_shared_view).
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
            let (start, len) = (range.start().raw(), range.length());
            // Collect replication targets first: (bytes, dst_space, dst_addr).
            let mut replications: Vec<(Vec<u8>, AddressSpaceId, u64)> = Vec::new();
            for mapping in self.spaces.shared.values() {
                let source_view = mapping
                    .views
                    .iter()
                    .find(|(space, base)| {
                        *space == source_space
                            && start >= *base
                            && start + len <= *base + mapping.size
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
                        // An unreadable source range cannot have
                        // committed (a committed write reads back), so
                        // there is nothing to replicate. Reachable
                        // only when the caller passes effects that
                        // never landed, e.g. a fault-discarded batch.
                        None => continue,
                    }
                };
                for &(dst_space, dst_base) in &mapping.views {
                    // Skip only the exact view the write landed in: a
                    // second view in the same space is still an alias
                    // of the shared bytes and must receive the write.
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
                let (mem, dst_reservations) = match dst_space {
                    AddressSpaceId::BOOT => (&mut self.memory, &mut self.reservations),
                    s => (
                        self.spaces
                            .extra
                            .get_mut(&s)
                            .expect("views validated at registration"),
                        self.spaces
                            .extra_reservations
                            .get_mut(&s)
                            .expect("reservation table is created with its space"),
                    ),
                };
                mem.apply_commit(dst_range, &bytes)
                    .expect("sibling view region installed at registration");
                // No holder is exempt: a store to the shared bytes
                // invalidates every reservation covering an aliasing
                // range, the writer's own included.
                cleared += dst_reservations.clear_covering(dst_addr, len, None);
            }
        }
        cleared
    }

    /// Sibling-view aliases of `range` as seen from `unit`'s space:
    /// for every shared mapping whose view in that space contains
    /// `range`, the equivalent range through each other view. Empty
    /// when `range` touches no shared view. Dependency analysis uses
    /// this to keep cross-space writes to the same shared bytes from
    /// proving false independence.
    pub fn shared_alias_ranges(&self, unit: UnitId, range: ByteRange) -> Vec<ByteRange> {
        let space = self.spaces.space_of(unit);
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

    /// Committed-memory hash across every space's content. Mapping
    /// metadata is not folded here; it reaches the sync-channel state
    /// hash through `metadata_hash`.
    /// With no child spaces this is exactly space 0's content hash.
    /// Replay and exploration tooling compare schedules through this,
    /// not `memory()` alone, so cross-process divergence in a child
    /// space is witnessed.
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
}

#[cfg(test)]
#[path = "tests/spaces_tests.rs"]
mod tests;
