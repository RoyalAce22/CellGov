//! Scheduler-granted progress allowance and the cost units drawn against it.
//!
//! [`Budget`] is a remainder; [`InstructionCost`] is a draw against it.
//! `From<InstructionCost> for `[`super::GuestTicks`] is the only bridge
//! from consumed work to guest time.

use crate::GuestTicks;
use core::fmt;

/// Cost of a unit of work in retired guest instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, derive_more::Display)]
#[display("{_0}")]
pub struct InstructionCost(u64);

impl InstructionCost {
    /// No cost.
    pub const ZERO: Self = Self(0);

    /// Single retired instruction.
    pub const ONE: Self = Self(1);

    /// Lift a raw count.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying instruction count.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }
}

impl From<InstructionCost> for GuestTicks {
    /// One retired instruction equals one guest tick.
    #[inline]
    fn from(cost: InstructionCost) -> Self {
        GuestTicks::new(cost.raw())
    }
}

/// A remaining instruction allowance.
///
/// Serde shape: bare JSON number (`#[serde(transparent)]`). On-disk
/// fixtures store the raw count, not a `{"raw": N}` object, so the
/// wire format matches what `--budget N` accepts on the CLI.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(transparent)]
pub struct Budget(u64);

impl Budget {
    /// Exhausted; cannot cover any nonzero cost.
    pub const ZERO: Self = Self(0);

    /// Architectural default per-step grant: 256 retired instructions.
    pub const DEFAULT_STEP: Self = Self(256);

    /// Lift a raw amount.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying instruction count.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Whether this budget is fully consumed.
    #[inline]
    pub const fn is_exhausted(self) -> bool {
        self.0 == 0
    }

    /// Attempt to charge `cost` against this budget.
    #[must_use = "try_consume returns the new remainder; assign or pattern-match it"]
    #[inline]
    pub const fn try_consume(self, cost: InstructionCost) -> Consume {
        match self.0.checked_sub(cost.0) {
            Some(v) => Consume::Ok(Self(v)),
            None => Consume::Yield {
                remaining: self,
                shortfall: cost.0 - self.0,
            },
        }
    }

    /// Initial grant minus a remainder. Saturates at zero.
    #[must_use = "consumed_since returns the consumed amount; assign it"]
    #[inline]
    pub const fn consumed_since(self, remaining: Budget) -> InstructionCost {
        InstructionCost(self.0.saturating_sub(remaining.0))
    }
}

impl fmt::Display for Budget {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// Outcome of [`Budget::try_consume`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Consume {
    /// The cost fit. Carries the new remainder.
    Ok(Budget),
    /// The cost did not fit.
    Yield {
        /// Remainder unchanged from before the attempted consume.
        remaining: Budget,
        /// How much the cost exceeded the remainder.
        shortfall: u64,
    },
}

#[cfg(test)]
#[path = "tests/budget_tests.rs"]
mod tests;
