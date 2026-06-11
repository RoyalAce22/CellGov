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
//! [`RsxCallStack::state_hash`] is the equality witness for replay.
//! Derived [`PartialEq`] also compares stale bytes past `depth` and
//! is stricter than the hash. Snapshot/restore preserves stale bytes
//! via `Copy`.

use cellgov_mem::Fnv1aHasher;

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

/// Hash-input shape version; bump when [`RsxCallStack::state_hash`]
/// changes field order, count, endianness, or hasher family.
pub const CALL_STACK_HASH_FORMAT_VERSION: u8 = 1;

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

    /// Reset to pristine; the full `entries` zero matters for
    /// derived [`PartialEq`], not for `state_hash`.
    #[inline]
    pub fn clear(&mut self) {
        self.depth = 0;
        self.entries = [0u32; CALL_STACK_DEPTH];
    }

    /// FNV-1a digest of `(format_version, depth, entries[0..depth])`.
    /// Excludes the trailing slots so a stack at depth 2 hashes the
    /// same regardless of stale bytes in slots 2..[`CALL_STACK_DEPTH`].
    /// The version byte is written FIRST so a format bump
    /// invalidates every otherwise-identical stack uniformly.
    pub fn state_hash(&self) -> u64 {
        let mut h = Fnv1aHasher::new();
        h.write(&[CALL_STACK_HASH_FORMAT_VERSION]);
        h.write(&[self.depth]);
        for slot in 0..self.depth as usize {
            h.write(&self.entries[slot].to_le_bytes());
        }
        h.finish()
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
mod tests {
    use super::*;

    #[test]
    fn new_stack_is_empty() {
        let s = RsxCallStack::new();
        assert_eq!(s.depth(), 0);
        assert!(s.is_empty());
    }

    #[test]
    fn push_then_pop_round_trips_single_entry() {
        let mut s = RsxCallStack::new();
        assert!(s.push(0x1000).is_ok());
        assert_eq!(s.depth(), 1);
        assert!(!s.is_empty());
        assert_eq!(s.pop(), Ok(0x1000));
        assert!(s.is_empty());
    }

    #[test]
    fn push_pop_is_lifo() {
        let mut s = RsxCallStack::new();
        s.push(0x100).unwrap();
        s.push(0x200).unwrap();
        s.push(0x300).unwrap();
        assert_eq!(s.pop(), Ok(0x300));
        assert_eq!(s.pop(), Ok(0x200));
        assert_eq!(s.pop(), Ok(0x100));
        assert_eq!(s.pop(), Err(CallStackUnderflow));
    }

    #[test]
    fn push_at_capacity_returns_overflow_without_mutating() {
        let mut s = RsxCallStack::new();
        for i in 0..CALL_STACK_DEPTH {
            s.push(i as u32).unwrap();
        }
        let pre = s;
        assert_eq!(s.push(0xDEAD), Err(CallStackOverflow));
        assert_eq!(s, pre, "rejected push must not perturb stack state");
    }

    #[test]
    fn pop_on_empty_returns_underflow() {
        let mut s = RsxCallStack::new();
        assert_eq!(s.pop(), Err(CallStackUnderflow));
    }

    #[test]
    fn overflow_and_underflow_are_distinct_typed_channels() {
        // Compile-time witness: the two error types do not unify, so
        // a caller cannot collapse them without explicit conversion.
        fn is_overflow(_: CallStackOverflow) -> &'static str {
            "overflow"
        }
        fn is_underflow(_: CallStackUnderflow) -> &'static str {
            "underflow"
        }
        let mut s = RsxCallStack::new();
        match s.pop() {
            Err(e) => assert_eq!(is_underflow(e), "underflow"),
            Ok(_) => panic!("empty stack must underflow"),
        }
        for i in 0..CALL_STACK_DEPTH {
            s.push(i as u32).unwrap();
        }
        match s.push(0xCAFE) {
            Err(e) => assert_eq!(is_overflow(e), "overflow"),
            Ok(()) => panic!("full stack must overflow"),
        }
    }

    #[test]
    fn clear_resets_to_pristine_state_hash() {
        let mut s = RsxCallStack::new();
        s.push(0x1234).unwrap();
        s.push(0x5678).unwrap();
        s.clear();
        let fresh = RsxCallStack::new();
        assert_eq!(s, fresh);
        assert_eq!(s.state_hash(), fresh.state_hash());
    }

    #[test]
    fn state_hash_ignores_stale_slots_past_depth() {
        let mut a = RsxCallStack::new();
        a.push(0x111).unwrap();
        a.push(0x222).unwrap();
        let mut b = RsxCallStack::new();
        b.push(0x111).unwrap();
        b.push(0x222).unwrap();
        b.push(0xDEAD).unwrap();
        b.pop().unwrap();
        assert_eq!(
            a.state_hash(),
            b.state_hash(),
            "depth-2 hash must not depend on bytes beneath the visible top",
        );
    }

    #[test]
    fn partial_eq_is_stricter_than_state_hash() {
        let mut a = RsxCallStack::new();
        a.push(0x111).unwrap();
        a.push(0xDEAD).unwrap();
        a.pop().unwrap();
        let mut b = RsxCallStack::new();
        b.push(0x111).unwrap();
        assert_eq!(
            a.state_hash(),
            b.state_hash(),
            "hash-equality holds modulo stale slots",
        );
        assert_ne!(a, b, "derived PartialEq surfaces the stale-byte difference");
    }

    #[test]
    fn state_hash_distinguishes_depths() {
        let mut a = RsxCallStack::new();
        a.push(0x100).unwrap();
        let mut b = RsxCallStack::new();
        b.push(0x100).unwrap();
        b.push(0x200).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_entry_values() {
        let mut a = RsxCallStack::new();
        a.push(0x100).unwrap();
        let mut b = RsxCallStack::new();
        b.push(0x200).unwrap();
        assert_ne!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn empty_stack_state_hash_golden() {
        // Doubles as a fixed-seed canary for `cellgov_mem::Fnv1aHasher`:
        // stable only if `Fnv1aHasher::new()` uses the FNV offset basis
        // with no host or run seeding.
        const EXPECTED: u64 = 0x082f_2207_b4e8_8cc4;
        let actual = RsxCallStack::new().state_hash();
        assert_eq!(
            actual, EXPECTED,
            "empty-stack hash drift: got 0x{actual:016x}, expected 0x{EXPECTED:016x}; \
             if CALL_STACK_HASH_FORMAT_VERSION was bumped this golden must move too",
        );
    }

    #[test]
    fn single_entry_state_hash_golden() {
        const EXPECTED: u64 = 0xff54_00f1_b413_16bc;
        let mut s = RsxCallStack::new();
        s.push(0x100).unwrap();
        let actual = s.state_hash();
        assert_eq!(
            actual, EXPECTED,
            "depth-1 hash drift: got 0x{actual:016x}, expected 0x{EXPECTED:016x}; \
             field-order or entry-encoding drift in state_hash",
        );
    }
}
