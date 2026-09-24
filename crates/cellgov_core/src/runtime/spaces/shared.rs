//! Process-shared mappings: registration, keyed attach and promotion, the segment copy, and alias ranges.

use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, MemError, PageSize};
use cellgov_trace::HostWriter;

use crate::runtime::state::Runtime;

use super::table::{AddressSpaceId, SharedMapping, SpaceError};

impl Runtime {
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
    /// A mapping registers in [`SpaceTable::shared`](super::table::SpaceTable::shared) when a second
    /// space attaches. Promotion adopts every recorded view and seeds
    /// each from the first, because an attach observes the content
    /// written before it and a repeat map inside the first space holds
    /// its own zero-filled region. A later attach appends to the live
    /// mapping the same way.
    pub(in crate::runtime) fn attach_keyed_shm_view(
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
}

impl Runtime {
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
}

#[cfg(test)]
#[path = "tests/shared_view_store_tests.rs"]
mod shared_view_store_tests;
