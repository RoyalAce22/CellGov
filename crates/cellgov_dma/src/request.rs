//! Immutable DMA request packet and direction enum.
//!
//! Completion timing is decided later by an implementation of
//! [`crate::DmaLatencyModel`]; the transfer itself is applied through the
//! commit pipeline.

use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::hw::spu::MfcTagId;

/// Direction of a modeled DMA transfer.
///
/// Variant order is part of the determinism contract for any containing
/// type that derives `Ord`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(u8)]
pub enum DmaDirection {
    /// Destination range is globally visible (SPU `put` shape).
    Put = 0,
    /// Source range is globally visible (SPU `get` shape).
    Get = 1,
}

/// How a queued command orders against its issuer's other queued
/// commands.
///
/// [CBEA p:69 s:7.9] a tag-specific fence orders a command after every preceding command in its tag group; a tag-specific barrier orders the command and every later command of its tag group after every preceding command in the group.
/// [CBEA p:308 s:Appendix D Table D-4] the barrier command orders every preceding nonimmediate command before every following command in the queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(u8)]
pub enum MfcOrdering {
    /// No ordering: the command may complete in any order.
    #[default]
    None = 0,
    /// A tag-specific fence.
    Fence = 1,
    /// A tag-specific barrier.
    TagBarrier = 2,
    /// The barrier command, over every command in the queue.
    QueueBarrier = 3,
}

/// An immutable DMA request packet.
///
/// Invariant: `source.length() == destination.length()`. Enforced by
/// [`DmaRequest::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DmaRequest {
    direction: DmaDirection,
    source: ByteRange,
    destination: ByteRange,
    issuer: UnitId,
    tag_id: Option<MfcTagId>,
    ordering: MfcOrdering,
    stall_notify: bool,
    holds_slot: bool,
    local_store_source: bool,
}

impl DmaRequest {
    /// Construct a `DmaRequest`.
    ///
    /// # Errors
    ///
    /// Returns `None` if `source.length() != destination.length()`. A
    /// zero-length transfer is permitted.
    #[inline]
    pub const fn new(
        direction: DmaDirection,
        source: ByteRange,
        destination: ByteRange,
        issuer: UnitId,
    ) -> Option<Self> {
        if source.length() != destination.length() {
            return None;
        }
        Some(Self {
            direction,
            source,
            destination,
            issuer,
            tag_id: None,
            ordering: MfcOrdering::None,
            stall_notify: false,
            holds_slot: true,
            local_store_source: false,
        })
    }

    /// Attach the MFC tag-id the SPU issued under. The request holds that
    /// tag group outstanding until it completes.
    #[inline]
    pub const fn with_tag_id(mut self, tag_id: MfcTagId) -> Self {
        self.tag_id = Some(tag_id);
        self
    }

    /// Attach the ordering the command's form sets.
    #[inline]
    pub const fn with_ordering(mut self, ordering: MfcOrdering) -> Self {
        self.ordering = ordering;
        self
    }

    /// The ordering the command's form sets.
    #[inline]
    pub const fn ordering(self) -> MfcOrdering {
        self.ordering
    }

    /// Mark the transfer as a list element with the stall-and-notify flag.
    ///
    /// [CBEA p:129 s:9.3.7] a list element with the stall-and-notify flag stalls its list after the element's transfer completes.
    #[inline]
    pub const fn with_stall_notify(mut self) -> Self {
        self.stall_notify = true;
        self
    }

    /// Whether the transfer is a list element whose list stalls once it
    /// completes.
    #[inline]
    pub const fn stall_notify(self) -> bool {
        self.stall_notify
    }

    /// Mark the transfer as a list element that holds no command-queue
    /// slot: one list command holds one slot, whatever its element count.
    #[inline]
    pub const fn without_slot(mut self) -> Self {
        self.holds_slot = false;
        self
    }

    /// Whether the transfer holds a slot in its issuer's command queue.
    #[inline]
    pub const fn holds_slot(self) -> bool {
        self.holds_slot
    }

    /// Mark a put's source as a range of its issuer's local store, which
    /// the runtime reads when the transfer completes.
    ///
    /// [CBEA p:173 s:10.3] the local-storage access of a queued command is complete when its tag group reads complete.
    #[inline]
    pub const fn with_local_store_source(mut self) -> Self {
        self.local_store_source = true;
        self
    }

    /// Whether the put reads its source from its issuer's local store at
    /// completion.
    #[inline]
    pub const fn local_store_source(self) -> bool {
        self.local_store_source
    }

    /// The range of the issuer's local store the transfer reads or
    /// writes at completion: a get's destination, or the source of a put
    /// from local store.
    #[inline]
    pub const fn local_store_range(self) -> Option<ByteRange> {
        match self.direction {
            DmaDirection::Get => Some(self.destination),
            DmaDirection::Put if self.local_store_source => Some(self.source),
            DmaDirection::Put => None,
        }
    }

    /// MFC tag-id the SPU issued under; `None` for PPU/host-initiated DMA.
    #[inline]
    pub const fn tag_id(self) -> Option<MfcTagId> {
        self.tag_id
    }

    /// Direction of the transfer.
    #[inline]
    pub const fn direction(self) -> DmaDirection {
        self.direction
    }

    /// Source range; interpretation depends on [`Self::direction`].
    #[inline]
    pub const fn source(self) -> ByteRange {
        self.source
    }

    /// Destination range; interpretation depends on [`Self::direction`].
    #[inline]
    pub const fn destination(self) -> ByteRange {
        self.destination
    }

    /// Unit that issued the request. Used to route the completion wake
    /// event back to the right waiter.
    #[inline]
    pub const fn issuer(self) -> UnitId {
        self.issuer
    }

    /// The main-storage range the transfer writes: a put's destination.
    /// A get writes the issuer's local store, which is not main storage.
    #[inline]
    pub const fn main_storage_write(self) -> Option<ByteRange> {
        match self.direction {
            DmaDirection::Put => Some(self.destination),
            DmaDirection::Get => None,
        }
    }

    /// The main-storage range the transfer reads: a get's source, or a
    /// put's source when neither an inline payload nor local store holds
    /// its bytes.
    #[inline]
    pub const fn main_storage_read(self, payloaded: bool) -> Option<ByteRange> {
        match self.direction {
            DmaDirection::Get => Some(self.source),
            DmaDirection::Put if payloaded || self.local_store_source => None,
            DmaDirection::Put => Some(self.source),
        }
    }

    /// Length of the transfer in bytes.
    #[inline]
    pub const fn length(self) -> u64 {
        self.source.length()
    }
}

#[cfg(test)]
#[path = "tests/request_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/tag_bound_tests.rs"]
mod tag_bound_tests;
