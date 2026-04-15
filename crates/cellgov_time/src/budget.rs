//! Budget units -- a policy input granted to an execution unit at scheduling time.
//!
//! `Budget` is the scheduler's currency. It is granted to a unit when it is
//! scheduled and consumed as the unit makes progress. Consuming budget may
//! also consume guest ticks, but `Budget` and [`crate::ticks::GuestTicks`]
//! are distinct types and do not implicitly convert. The conversion policy
//! (uniform, weighted, ISA-specific) lives in the scheduler, not here.

/// A non-negative quantity of work a scheduler has authorized a unit to
/// perform before yielding.
///
/// `Budget` is a distinct type from [`crate::ticks::GuestTicks`] and
/// [`crate::epoch::Epoch`]: the three concepts must not implicitly convert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Budget(u64);

impl Budget {
    /// An empty budget. A unit holding `ZERO` budget cannot make further
    /// progress and must yield with `BudgetExhausted` on its next step.
    pub const ZERO: Self = Self(0);

    /// Construct a `Budget` from a raw amount.
    ///
    /// There is no `From<u64>` impl: every grant site must be explicit so
    /// that scheduler policy is auditable from the call graph.
    #[inline]
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    /// Return the underlying budget amount. Use sparingly -- prefer
    /// `is_exhausted` and `try_consume` over arithmetic on raw values.
    #[inline]
    pub const fn raw(self) -> u64 {
        self.0
    }

    /// Whether this budget is fully consumed.
    #[inline]
    pub const fn is_exhausted(self) -> bool {
        self.0 == 0
    }

    /// Attempt to consume `amount` from this budget, returning the remaining
    /// budget on success or `None` if the budget cannot cover the cost.
    ///
    /// A `None` here is the scheduler's signal that the unit must yield with
    /// `BudgetExhausted` -- it is not an error.
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
