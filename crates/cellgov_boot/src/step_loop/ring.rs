//! Fixed-capacity history rings a stepper keeps for its diagnostics.

use cellgov_core::AddressSpaceId;

const PC_RING_SIZE: usize = 64;
const SYSCALL_RING_SIZE: usize = 32;

/// Last `(space, PC)` pairs a stepper attempted; the space rides along
/// because equal PCs in different spaces never alias.
pub type PcRing = Ring<(AddressSpaceId, u64), PC_RING_SIZE>;
/// Last `(syscall number, call PC)` pairs a stepper dispatched.
pub type SyscallRing = Ring<(u64, u64), SYSCALL_RING_SIZE>;

/// Fixed-capacity overwrite ring: the newest entry replaces the
/// oldest once `N` are held.
#[derive(Debug, Clone, Copy)]
pub struct Ring<T, const N: usize> {
    entries: [Option<T>; N],
    cursor: RingCursor,
}

impl<T: Copy, const N: usize> Ring<T, N> {
    /// An empty ring. A zero `N` fails to compile.
    pub fn new() -> Self {
        // `RingCursor::record` wraps `pos` to 0 at capacity 0 and
        // hands back index 0 of an empty array; refuse at compile time.
        const { assert!(N > 0, "a Ring needs capacity for at least one entry") };
        Self {
            entries: [None; N],
            cursor: RingCursor::new(N),
        }
    }

    /// Record `entry`, which replaces the oldest once the ring is full.
    pub fn push(&mut self, entry: T) {
        let idx = self.cursor.record();
        self.entries[idx] = Some(entry);
    }

    /// Entries held, saturating at `N`.
    pub fn filled(&self) -> usize {
        self.cursor.filled()
    }

    /// Held entries oldest-to-newest.
    pub fn iter(&self) -> impl Iterator<Item = T> + '_ {
        self.cursor.iter_indices().map(move |i| {
            self.entries[i].expect("the cursor only yields indices a push has filled")
        })
    }
}

impl<T: Copy, const N: usize> Default for Ring<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Invariant: `pos` is always in `[0, capacity)`; `full` flips on first wrap.
#[derive(Debug, Clone, Copy)]
struct RingCursor {
    pos: usize,
    full: bool,
    capacity: usize,
}

impl RingCursor {
    fn new(capacity: usize) -> Self {
        Self {
            pos: 0,
            full: false,
            capacity,
        }
    }

    fn record(&mut self) -> usize {
        let idx = self.pos;
        self.pos += 1;
        if self.pos >= self.capacity {
            self.pos = 0;
            self.full = true;
        }
        idx
    }

    fn filled(&self) -> usize {
        if self.full {
            self.capacity
        } else {
            self.pos
        }
    }

    #[cfg(test)]
    fn is_full(&self) -> bool {
        self.full
    }

    /// Populated indices oldest-to-newest.
    fn iter_indices(&self) -> impl Iterator<Item = usize> + '_ {
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

#[cfg(test)]
#[path = "tests/ring_buffer_tests.rs"]
mod ring_buffer_tests;
