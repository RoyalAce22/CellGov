//! Replicating committed writes across the views of a shared segment.

use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_trace::HostWriter;

use crate::runtime::state::Runtime;

use super::table::AddressSpaceId;

impl Runtime {
    /// Replicate the committed writes that landed in a shared view into
    /// every sibling view, and return the reservations that cleared.
    ///
    /// Runs after a successful commit, inside the same batch boundary;
    /// mappings in key order, views in registration order. Each write
    /// spares its emitter's reservation; see
    /// [`Runtime::fanout_committed_range`].
    pub(in crate::runtime) fn fanout_shared_writes(
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
    pub(in crate::runtime) fn fanout_committed_range(
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
}

impl Runtime {
    /// Whether `range` lies inside any shared view of `space`.
    pub(in crate::runtime) fn range_intersects_shared_view(
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
}
