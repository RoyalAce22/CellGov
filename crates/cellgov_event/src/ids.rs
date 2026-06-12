//! Stable newtype identifiers for units, events, and sequence numbers.

/// A stable identifier for an execution unit.
///
/// Distinct from [`EventId`], [`SequenceNumber`], and raw `u64`.
/// Assigned by the `cellgov_core` unit registry; two live units must
/// never share an id within a single runtime instance.
///
/// Ordering is arbitrary-but-stable; `a < b` does not imply `a` was
/// registered first or is otherwise temporally prior. Provided so
/// `BTreeMap<UnitId, _>` works for deterministic per-unit storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UnitId(u64);

impl UnitId {
    /// Construct from a raw value. Production code receives ids from
    /// the registry; `new` is the trace-replay and scenario path.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying id value. Consumers: state hashing
    /// (`registry::runnable_hash`, `lv2_host::content_hash`),
    /// diagnostic printing, and cross-domain id translation
    /// (`UnitId` to `MailboxId` in dispatch paths).
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// A stable identifier for an event in the runtime's event log.
///
/// Distinct from [`UnitId`], [`SequenceNumber`], and raw `u64`. Not
/// part of [`crate::OrderingKey`]; used only for trace correlation.
///
/// Ordering is arbitrary-but-stable; `a < b` does not imply event
/// `a` happened before event `b`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EventId(u64);

impl EventId {
    /// Construct from a raw value; trace-replay and scenario path.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying id value. Consumers: trace serialization and
    /// diagnostic printing.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Final tie-break counter for [`crate::OrderingKey`].
///
/// Distinct from [`UnitId`], [`EventId`], and raw `u64`. Overflow is
/// an invariant violation: [`SequenceNumber::next`] returns `None`
/// at `u64::MAX` rather than wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SequenceNumber(u64);

impl SequenceNumber {
    /// The first sequence number issued by a fresh runtime.
    pub const ZERO: Self = Self(0);

    /// Construct from a raw value; trace-replay path.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying counter value. Consumers: trace wire format,
    /// `OrderingKey` packing, and diagnostic printing.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// The successor sequence number, or `None` on overflow.
    #[inline]
    pub const fn next(self) -> Option<Self> {
        match self.0.checked_add(1) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
}

#[cfg(test)]
#[path = "tests/ids_tests.rs"]
mod tests;
