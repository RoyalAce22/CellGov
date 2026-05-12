pub(in crate::game) const PC_RING_SIZE: usize = 64;
pub(in crate::game) const SYSCALL_RING_SIZE: usize = 32;

/// Invariant: `pos` is always in `[0, capacity)`; `full` flips on first wrap.
#[derive(Debug, Clone, Copy)]
pub(in crate::game) struct RingCursor {
    pos: usize,
    full: bool,
    capacity: usize,
}

impl RingCursor {
    pub(in crate::game) fn new(capacity: usize) -> Self {
        Self {
            pos: 0,
            full: false,
            capacity,
        }
    }

    pub(in crate::game) fn record(&mut self) -> usize {
        let idx = self.pos;
        self.pos += 1;
        if self.pos >= self.capacity {
            self.pos = 0;
            self.full = true;
        }
        idx
    }

    pub(in crate::game) fn filled(&self) -> usize {
        if self.full {
            self.capacity
        } else {
            self.pos
        }
    }

    #[allow(dead_code)]
    pub(in crate::game) fn is_full(&self) -> bool {
        self.full
    }

    /// Populated indices oldest-to-newest.
    pub(in crate::game) fn iter_indices(&self) -> impl Iterator<Item = usize> + '_ {
        let (a_start, a_end, b_start, b_end) = if self.full {
            (self.pos, self.capacity, 0, self.pos)
        } else {
            (0, self.pos, 0, 0)
        };
        (a_start..a_end).chain(b_start..b_end)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_cursor_records_in_order_until_full() {
        let mut c = RingCursor::new(4);
        assert_eq!(c.record(), 0);
        assert_eq!(c.record(), 1);
        assert_eq!(c.filled(), 2);
        assert!(!c.is_full());
    }

    #[test]
    fn ring_cursor_wraps_and_marks_full() {
        let mut c = RingCursor::new(3);
        c.record();
        c.record();
        c.record();
        assert!(c.is_full());
        assert_eq!(c.filled(), 3);
        assert_eq!(c.record(), 0);
        assert_eq!(c.filled(), 3);
        assert!(c.is_full());
    }

    #[test]
    fn ring_cursor_iter_indices_partial_yields_in_order() {
        let mut c = RingCursor::new(4);
        c.record();
        c.record();
        let v: Vec<_> = c.iter_indices().collect();
        assert_eq!(v, vec![0, 1]);
    }

    #[test]
    fn ring_cursor_iter_indices_full_yields_oldest_first() {
        let mut c = RingCursor::new(3);
        for _ in 0..5 {
            c.record();
        }
        let v: Vec<_> = c.iter_indices().collect();
        assert_eq!(v, vec![2, 0, 1]);
    }

    #[test]
    fn ring_cursor_iter_indices_empty_yields_nothing() {
        let c = RingCursor::new(4);
        assert_eq!(c.iter_indices().count(), 0);
    }
}
