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
#[path = "tests/ring_tests.rs"]
mod tests;
