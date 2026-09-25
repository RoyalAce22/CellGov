//! Fixed-depth return-address stack used by `rsx_advance` when
//! honoring `Call` / `Return` FIFO control headers. Snapshot-captured
//! alongside `RsxFifoCursor`.
//!
//! Two faults surface via `rsx_advance` as `RsxAdvanceStop::Malformed`
//! with distinct synthetic raws:
//! - Call overflow ([`CALL_STACK_OVERFLOW_RAW`]) from
//!   [`RsxCallStack::push`] on a full stack.
//! - Return underflow
//!   ([`crate::rsx::advance::RSX_ADVANCE_UNDERFLOW_RAW`]) from
//!   [`RsxCallStack::pop`] on an empty stack.
//!
//! ## Determinism contract
//!
//! [`RsxCallStack::sync_term`] is the equality witness for replay.
//! Derived [`PartialEq`] also compares stale bytes past `depth` and
//! is stricter than the term. Snapshot/restore preserves stale bytes
//! via `Copy`.

/// Maximum simultaneous Call/Return nesting before
/// [`RsxCallStack::push`] returns [`CallStackOverflow`]. Heuristic
/// cap; `CALL_STACK_OVERFLOW_RAW` distinguishes the cap from a real
/// malformed-header rejection at the fault site.
pub const CALL_STACK_DEPTH: usize = 8;

// `depth: u8` smuggles the `CALL_STACK_DEPTH <= u8::MAX` invariant.
const _: () = assert!(
    CALL_STACK_DEPTH <= u8::MAX as usize,
    "depth: u8 cannot hold CALL_STACK_DEPTH; widen `depth` or lower the cap",
);

/// Synthetic raw word emitted as `Malformed { raw }` when
/// [`RsxCallStack::push`] reports overflow.
pub const CALL_STACK_OVERFLOW_RAW: u32 = 0x4000_00FF;

/// Fixed-depth return-address stack; each entry is the byte offset
/// the FIFO walker resumes at after the matching `Return`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxCallStack {
    entries: [u32; CALL_STACK_DEPTH],
    depth: u8,
}

impl Default for RsxCallStack {
    fn default() -> Self {
        Self::new()
    }
}

impl RsxCallStack {
    /// Pristine empty stack.
    #[inline]
    pub const fn new() -> Self {
        Self {
            entries: [0u32; CALL_STACK_DEPTH],
            depth: 0,
        }
    }

    /// Current stack depth (0..=[`CALL_STACK_DEPTH`]).
    #[inline]
    pub const fn depth(&self) -> u8 {
        self.depth
    }

    /// True when no Call is currently pending a Return.
    #[inline]
    pub const fn is_empty(&self) -> bool {
        self.depth == 0
    }

    /// Push a return address. Returns [`CallStackOverflow`] on a
    /// full stack without mutating.
    #[inline]
    pub fn push(&mut self, return_addr: u32) -> Result<(), CallStackOverflow> {
        let slot = self.depth as usize;
        if slot >= CALL_STACK_DEPTH {
            return Err(CallStackOverflow);
        }
        self.entries[slot] = return_addr;
        self.depth += 1;
        debug_assert!(
            self.depth as usize <= CALL_STACK_DEPTH,
            "depth invariant: post-push depth ({}) exceeded CALL_STACK_DEPTH ({}); \
             the slot >= CAP guard must precede every increment",
            self.depth,
            CALL_STACK_DEPTH,
        );
        Ok(())
    }

    /// Pop the most recent return address. Returns
    /// [`CallStackUnderflow`] on an empty stack.
    #[inline]
    pub fn pop(&mut self) -> Result<u32, CallStackUnderflow> {
        if self.depth == 0 {
            return Err(CallStackUnderflow);
        }
        self.depth -= 1;
        Ok(self.entries[self.depth as usize])
    }

    /// Reset to pristine. Only derived [`PartialEq`] reads the zeroed
    /// slots above the depth.
    #[inline]
    pub fn clear(&mut self) {
        self.depth = 0;
        self.entries = [0u32; CALL_STACK_DEPTH];
    }

    /// The stack's term of the sync-state sum, computed on read.
    pub fn sync_term(&self) -> u128 {
        cellgov_mem::lanes::value_term(cellgov_mem::lanes::source::RSX_CALL_STACK, 0, self)
    }
}

/// Field 1 holds the depth, and slot `i` of field 2 holds entry `i`.
///
/// Only the live entries add a lane, so stale bytes above the depth
/// leave the term unchanged.
impl cellgov_mem::lanes::LaneValue for RsxCallStack {
    fn lanes(&self, lanes: &mut cellgov_mem::lanes::ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.depth));
        for (slot, entry) in self.entries[..self.depth as usize].iter().enumerate() {
            lanes.lane(2, slot as u64, u64::from(*entry));
        }
    }
}

/// Overflow signal returned by [`RsxCallStack::push`] when the
/// stack is already at [`CALL_STACK_DEPTH`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("RSX call stack overflow at depth {CALL_STACK_DEPTH}")]
pub struct CallStackOverflow;

/// Underflow signal returned by [`RsxCallStack::pop`] when the
/// stack is empty.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("RSX call stack underflow: Return decoded with empty stack")]
pub struct CallStackUnderflow;

#[cfg(test)]
#[path = "tests/call_stack_tests.rs"]
mod tests;
