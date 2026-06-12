//! FIFO queue of PPU threads parked on a single primitive. Wake order
//! is strictly enqueue order.

use std::collections::VecDeque;

use crate::ppu_thread::PpuThreadId;
use crate::sync_primitives::errors::DuplicateEnqueue;

/// FIFO queue of PPU threads parked on a single primitive.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WaiterList {
    queue: VecDeque<PpuThreadId>,
}

impl WaiterList {
    /// Construct an empty list.
    pub fn new() -> Self {
        Self {
            queue: VecDeque::new(),
        }
    }

    /// Append `id` to the tail.
    ///
    /// # Errors
    /// [`DuplicateEnqueue`] if `id` is already parked.
    #[must_use = "duplicate enqueue is a programming error; the caller must handle it"]
    pub fn enqueue(&mut self, id: PpuThreadId) -> Result<(), DuplicateEnqueue> {
        if self.queue.iter().any(|&existing| existing == id) {
            return Err(DuplicateEnqueue { id });
        }
        self.queue.push_back(id);
        Ok(())
    }

    /// Pop the head, or `None` if empty.
    pub fn dequeue_one(&mut self) -> Option<PpuThreadId> {
        self.queue.pop_front()
    }

    /// Drain all waiters in enqueue order.
    pub fn drain_all(&mut self) -> std::collections::vec_deque::Drain<'_, PpuThreadId> {
        self.queue.drain(..)
    }

    /// Whether `id` is currently parked.
    pub fn contains(&self, id: PpuThreadId) -> bool {
        self.queue.iter().any(|&existing| existing == id)
    }

    /// Remove `id`, preserving the order of the remaining waiters.
    pub fn remove(&mut self, id: PpuThreadId) -> bool {
        if let Some(pos) = self.queue.iter().position(|&existing| existing == id) {
            self.queue.remove(pos);
            true
        } else {
            false
        }
    }

    /// Number of parked waiters.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the list is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Read-only iterator in enqueue order.
    pub fn iter(&self) -> impl Iterator<Item = PpuThreadId> + '_ {
        self.queue.iter().copied()
    }
}

#[cfg(test)]
#[path = "tests/waiter_list_tests.rs"]
mod tests;
