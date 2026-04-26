//! Scheduler-granted progress allowance and the cost units drawn against it.
//!
//! [`Budget`] is a remainder; [`InstructionCost`] is a draw against it.
//! `From<InstructionCost> for `[`super::GuestTicks`] is the only bridge
//! from consumed work to guest time.

use crate::GuestTicks;
use core::fmt;

/// Cost of a unit of work in retired guest instructions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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

impl fmt::Display for InstructionCost {
    #[inline]
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// A remaining instruction allowance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
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
mod tests {
    use super::*;

    #[test]
    fn zero_is_exhausted() {
        assert!(Budget::ZERO.is_exhausted());
        assert_eq!(Budget::ZERO.raw(), 0);
    }

    #[test]
    fn nonzero_is_not_exhausted() {
        assert!(!Budget::new(1).is_exhausted());
    }

    #[test]
    fn default_step_is_256() {
        assert_eq!(Budget::DEFAULT_STEP.raw(), 256);
    }

    #[test]
    fn try_consume_returns_ok_within_budget() {
        let b = Budget::new(100);
        assert_eq!(
            b.try_consume(InstructionCost::new(40)),
            Consume::Ok(Budget::new(60)),
        );
    }

    #[test]
    fn try_consume_exact_yields_zero_budget() {
        let b = Budget::new(100);
        assert_eq!(
            b.try_consume(InstructionCost::new(100)),
            Consume::Ok(Budget::ZERO),
        );
    }

    #[test]
    fn try_consume_overdraw_returns_yield_with_shortfall() {
        let b = Budget::new(10);
        assert_eq!(
            b.try_consume(InstructionCost::new(11)),
            Consume::Yield {
                remaining: Budget::new(10),
                shortfall: 1,
            },
        );
    }

    #[test]
    fn try_consume_zero_cost_against_exhausted_does_not_yield() {
        assert_eq!(
            Budget::ZERO.try_consume(InstructionCost::ZERO),
            Consume::Ok(Budget::ZERO),
        );
    }

    #[test]
    fn try_consume_zero_cost_preserves_remainder() {
        let b = Budget::new(7);
        assert_eq!(b.try_consume(InstructionCost::ZERO), Consume::Ok(b));
    }

    #[test]
    fn try_consume_at_u64_max_boundary() {
        let b = Budget::new(u64::MAX);
        assert_eq!(
            b.try_consume(InstructionCost::new(u64::MAX)),
            Consume::Ok(Budget::ZERO),
        );
    }

    #[test]
    fn try_consume_overdraw_by_huge_amount() {
        let b = Budget::new(1);
        assert_eq!(
            b.try_consume(InstructionCost::new(u64::MAX)),
            Consume::Yield {
                remaining: Budget::new(1),
                shortfall: u64::MAX - 1,
            },
        );
    }

    #[test]
    fn is_exhausted_after_full_consume() {
        let b = Budget::new(5);
        match b.try_consume(InstructionCost::new(5)) {
            Consume::Ok(after) => assert!(after.is_exhausted()),
            Consume::Yield { .. } => panic!("exact consume should be Ok"),
        }
    }

    #[test]
    fn consumed_since_invariant() {
        let initial = Budget::new(256);
        let remaining = Budget::new(73);
        let consumed = initial.consumed_since(remaining);
        assert_eq!(consumed.raw() + remaining.raw(), initial.raw());
    }

    #[test]
    fn consumed_since_at_zero_remaining_is_full_grant() {
        let initial = Budget::DEFAULT_STEP;
        assert_eq!(
            initial.consumed_since(Budget::ZERO),
            InstructionCost::new(256),
        );
    }

    #[test]
    fn consumed_since_at_full_remaining_is_zero() {
        let initial = Budget::new(256);
        assert_eq!(
            initial.consumed_since(Budget::new(256)),
            InstructionCost::ZERO,
        );
    }

    #[test]
    fn consumed_since_saturates_on_inverted_inputs() {
        assert_eq!(
            Budget::new(10).consumed_since(Budget::new(20)),
            InstructionCost::ZERO,
        );
    }

    #[test]
    fn ordering_is_total() {
        assert!(Budget::new(1) < Budget::new(2));
        assert_eq!(Budget::new(7), Budget::new(7));
    }

    #[test]
    fn instruction_cost_constants() {
        assert_eq!(InstructionCost::ZERO.raw(), 0);
        assert_eq!(InstructionCost::ONE.raw(), 1);
    }

    #[test]
    fn instruction_cost_to_guest_ticks_is_identity() {
        let cost = InstructionCost::new(256);
        let ticks: GuestTicks = cost.into();
        assert_eq!(ticks, GuestTicks::new(256));
    }

    #[test]
    fn budget_display_is_bare_number() {
        assert_eq!(format!("{}", Budget::ZERO), "0");
        assert_eq!(format!("{}", Budget::DEFAULT_STEP), "256");
        assert_eq!(
            format!("{}", Budget::new(u64::MAX)),
            format!("{}", u64::MAX)
        );
    }

    #[test]
    fn instruction_cost_display_is_bare_number() {
        assert_eq!(format!("{}", InstructionCost::ZERO), "0");
        assert_eq!(format!("{}", InstructionCost::ONE), "1");
        assert_eq!(format!("{}", InstructionCost::new(42)), "42");
    }

    #[test]
    fn hot_loop_drains_default_step_exactly() {
        let mut b = Budget::DEFAULT_STEP;
        for _ in 0..256 {
            match b.try_consume(InstructionCost::ONE) {
                Consume::Ok(after) => b = after,
                Consume::Yield { .. } => panic!("budget yielded before 256 charges"),
            }
        }
        assert!(b.is_exhausted());
        assert_eq!(
            b.try_consume(InstructionCost::ONE),
            Consume::Yield {
                remaining: Budget::ZERO,
                shortfall: 1,
            },
        );
    }
}
