//! Priority class enum -- second tier of [`crate::OrderingKey`].

/// Coarse priority class for events flowing through the runtime.
///
/// Lower numeric value = higher priority
/// [CBE-Handbook p:265 s:9.6.2.2 Current Priority Level], so a
/// min-heap or `BTreeMap` keyed by [`crate::OrderingKey`] pops the
/// highest-priority class first.
///
/// Variant order and `#[repr(u8)]` discriminants are part of the
/// determinism contract; a reorder silently changes every tie-break.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, strum::VariantArray,
)]
#[repr(u8)]
pub enum PriorityClass {
    /// Runtime-essential events that must not be reordered behind
    /// any other class within the same timestamp.
    Critical = 0,
    /// Latency-sensitive but not critical.
    High = 1,
    /// The default class.
    #[default]
    Normal = 2,
    /// Best-effort work that may be deferred behind anything else.
    Background = 3,
}

#[cfg(test)]
#[path = "tests/priority_tests.rs"]
mod tests;
