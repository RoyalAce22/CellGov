//! Scheduler-granted progress allowance for an execution unit.

/// Work a scheduler has authorized a unit to perform before yielding.
///
/// Distinct from [`crate::ticks::GuestTicks`] and [`crate::epoch::Epoch`];
/// the three do not implicitly convert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Budget(u64);

impl Budget {
    /// A unit holding `ZERO` must yield with `BudgetExhausted` on its next step.
    pub const ZERO: Self = Self(0);

    /// Lift a raw amount into a `Budget`.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Underlying budget amount.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Whether this budget is fully consumed.
    #[inline]
    pub const fn is_exhausted(self) -> bool {
        self.0 == 0
    }

    /// Consume `amount`, returning the remainder, or `None` if the budget
    /// cannot cover the cost.
    ///
    /// `None` is the scheduler's yield signal, not an error.
    #[inline]
    pub const fn try_consume(self, amount: Budget) -> Option<Budget> {
        match self.0.checked_sub(amount.0) {
            Some(v) => Some(Self(v)),
            None => None,
        }
    }
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
    fn try_consume_within_budget() {
        let b = Budget::new(100);
        assert_eq!(b.try_consume(Budget::new(40)), Some(Budget::new(60)));
    }

    #[test]
    fn try_consume_exact_yields_zero() {
        let b = Budget::new(100);
        assert_eq!(b.try_consume(Budget::new(100)), Some(Budget::ZERO));
    }

    #[test]
    fn try_consume_overdraw_yields_none() {
        let b = Budget::new(10);
        assert_eq!(b.try_consume(Budget::new(11)), None);
    }

    #[test]
    fn ordering_is_total() {
        assert!(Budget::new(1) < Budget::new(2));
        assert_eq!(Budget::new(7), Budget::new(7));
    }
}
