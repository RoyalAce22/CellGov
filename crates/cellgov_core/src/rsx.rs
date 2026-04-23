//! RSX CPU-side completion state.
//!
//! The PS3 RSX exposes a FIFO command buffer in guest memory. The guest
//! writes NV method commands and advances a `put` pointer at the RSX
//! control register (`0xC0000040`); the GPU consumes commands and
//! advances `get` (`0xC0000044`). Games poll labels, flip-status, and
//! `SetReport` results for completion.
//!
//! In CellGov there is no real GPU. The "advance pass" that consumes
//! methods is a deterministic state machine run by the commit pipeline
//! at each commit boundary: it reads the FIFO region (frozen view),
//! decodes method headers, and emits completion effects (label writes,
//! flip transitions). Completion timing is deterministic because
//! commit-boundary ordering is deterministic; CellGov's fidelity claim
//! is "the value the CPU polls is the deterministic CPU-visible
//! completion value defined by the commit-boundary model," not "the
//! value a real GPU would have written at this wall-clock instant."
//!
//! This module holds the pure-data committed state the advance pass
//! operates on. It is deliberately dumb: a triple of raw `u32` byte
//! offsets with typed accessors. It does NOT know the ring-buffer size,
//! does NOT compute drain conditions, and does NOT enforce any
//! ordering invariant between `put` and `get` -- those are all the
//! advance pass's job, because they depend on the FIFO geometry the
//! cursor is too narrow to see.

pub mod advance;
pub mod flip;
pub mod method;
pub mod reports;

/// Guest address of the RSX control register's `put` slot. Writes from
/// the guest here are mirrored into [`RsxFifoCursor::put`].
pub const RSX_CONTROL_PUT_ADDR: u32 = 0xC000_0040;

/// Guest address of the RSX control register's `get` slot. The advance
/// pass updates this to reflect how far it has drained.
pub const RSX_CONTROL_GET_ADDR: u32 = 0xC000_0044;

/// Guest address of the RSX control register's `reference` slot. The
/// advance pass writes here when an `NV406E_SET_REFERENCE` method is
/// consumed; the guest reads via `cellGcmGetCurrentReference`.
pub const RSX_CONTROL_REF_ADDR: u32 = 0xC000_0048;

/// Guest address at which the oracle mirrors the current flip-status
/// byte whenever `rsx_flip.status` changes. Real PS3 libgcm exposes
/// the flip status through a label the game polls after issuing
/// `cellGcmSetFlip`; guests that skip cellGcmInit (microtests, and
/// any other caller that drives the RSX cursor directly) cannot
/// compute that label address, so the oracle additionally maintains
/// a fixed-address mirror at this location. Writes happen ONLY on
/// transitions, so a guest reading this address observes each
/// status change exactly once (no tearing, no lost updates). Written
/// as a 4-byte big-endian u32 so a native PPU load lifts the status
/// into the low byte; the upper 24 bits are zero.
pub const RSX_FLIP_STATUS_MIRROR_ADDR: u32 = 0xC000_0050;

/// Format version byte prefixed into [`RsxFifoCursor::state_hash`].
///
/// Bump this whenever the hash input shape changes -- field order,
/// field count, endianness, or hasher family. A change here means
/// historical traces are intentionally incomparable with post-change
/// traces; the bump surfaces that at the golden-test boundary so the
/// comparison tooling never silently accepts a mismatched shape as a
/// "memory state differs" divergence.
///
/// Bidirectional coupling: when this constant changes, the
/// `empty_cursor_hash_golden` test must fail until the expected value
/// is updated. When the golden value is updated without bumping this
/// constant, the test still fails -- the constant is hashed first,
/// so any input-shape change without a version bump produces a
/// different golden.
pub const STATE_HASH_FORMAT_VERSION: u8 = 1;

/// Put / get / reference triple backing the RSX FIFO.
///
/// The cursor stores three raw u32 values -- the guest-visible
/// byte-offset components of the RSX control register's put / get /
/// reference slots. It does NOT interpret those offsets against a
/// specific ring size; callers (the RSX IO region writeback path and
/// the FIFO advance pass) own that knowledge.
///
/// ### Why the cursor is deliberately permissive
///
/// Every mutator accepts any `u32`. Concretely:
///
/// - [`Self::set_put`] stores whatever the guest wrote, including a
///   value smaller than the current `get` (a "wrap reset"), a value
///   beyond any legal ring offset (a pathological guest), or a
///   repeated value (idempotent re-advance). The state hash captures
///   the raw stored value because that IS the guest-observable
///   committed state: the guest reads back put from
///   `RSX_CONTROL_PUT_ADDR` and sees whatever it wrote, not a masked
///   variant. Masking on set would make two observably-different
///   guest writes collapse into the same committed state, which is a
///   correctness hazard for divergence tracing.
/// - [`Self::set_get`] accepts any `u32`. The advance pass computes
///   the next `get` value with full ring-wrap logic (modulo ring
///   size) and writes it in full. The cursor does not check
///   `new_get <= put` because a legitimate wrap may produce
///   `new_get < get`, which is not an error -- the assertion
///   belongs in the advance pass, where the ring size is known.
/// - [`Self::set_reference`] accepts any value, including zero.
///   Zero is both the default and a legal reference value a guest
///   can write; the cursor does not distinguish "never set" from
///   "set to zero," matching real PS3 semantics where the initial
///   reference IS zero and guests polling for reference=0 correctly
///   see success on a pristine control register.
///
/// ### What the cursor does NOT provide
///
/// - No `has_pending()`. Drain condition is ring-size-dependent.
/// - No `advance_get(delta)`. Wrap arithmetic belongs to the caller.
/// - No put/get ordering assertion. The cursor has no way to know
///   what a legal post-wrap relationship looks like.
/// - No backward-put auto-reset. A `set_put` with a value less than
///   the current `get` does not reset `get`. If the advance pass
///   wants to drain the pre-wrap tail first and then reset, it
///   calls `set_get` explicitly at the appropriate moment.
///
/// These omissions are the point. Every enforcement that requires
/// ring-buffer semantics lives in the advance pass.
///
/// ### Savestate and restore
///
/// The cursor's reason for existing is that it folds into the
/// committed memory-state hash for deterministic replay. A future
/// savestate / restore facility will need to rebuild the cursor to
/// a captured mid-execution value; that path IS a legitimate
/// invoker of all three mutators with values that did not come from
/// the guest or the advance pass.
///
/// The cursor is safe for this because no mutator has cross-field
/// side effects: `set_put`, `set_get`, and `set_reference` each
/// touch exactly their one field, so the order of restoration does
/// not matter. A savestate may restore the three fields in any
/// order and the resulting cursor is byte-identical to the captured
/// cursor. The "backward set_put does not auto-reset get" rule is
/// what makes field-order-independent restore safe -- if `set_put`
/// secretly modified `get` under some condition, restoring `put`
/// before `get` would corrupt the captured state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RsxFifoCursor {
    put: u32,
    get: u32,
    current_reference: u32,
}

impl RsxFifoCursor {
    /// Construct an empty cursor: put, get, and reference all zero.
    #[inline]
    pub const fn new() -> Self {
        Self {
            put: 0,
            get: 0,
            current_reference: 0,
        }
    }

    /// Current put pointer (mirror of [`RSX_CONTROL_PUT_ADDR`]).
    #[inline]
    pub const fn put(self) -> u32 {
        self.put
    }

    /// Current get pointer (mirror of [`RSX_CONTROL_GET_ADDR`]).
    #[inline]
    pub const fn get(self) -> u32 {
        self.get
    }

    /// Current reference value (mirror of [`RSX_CONTROL_REF_ADDR`]).
    ///
    /// The initial value is zero, matching real PS3 RSX init
    /// semantics. A poll for reference=0 on a pristine cursor
    /// succeeds, which is also the real-hardware behaviour.
    #[inline]
    pub const fn current_reference(self) -> u32 {
        self.current_reference
    }

    /// Store a new put value verbatim. See struct-level docs for the
    /// permissiveness rationale.
    ///
    /// Common callers: the RSX IO region writeback path invoked when
    /// the guest writes to [`RSX_CONTROL_PUT_ADDR`].
    #[inline]
    pub fn set_put(&mut self, put: u32) {
        self.put = put;
    }

    /// Store a new get value verbatim. The advance pass owns the
    /// wrap-handling logic and writes the computed post-wrap value
    /// here in full.
    ///
    /// No assertion against `put`. A legitimate ring wrap produces
    /// `new_get < get` transiently; any invariant check belongs in
    /// the advance pass where the ring size is known.
    ///
    /// **Legitimate callers:** the FIFO advance pass (normal drain)
    /// and the savestate-restore path (rebuild captured cursor).
    /// No other code should invoke this -- a stray `set_get` from
    /// elsewhere corrupts the invariant the advance pass is meant
    /// to maintain. Rust cannot enforce this with types without
    /// module-private capability tokens; the rule is a review
    /// discipline.
    #[inline]
    pub fn set_get(&mut self, get: u32) {
        self.get = get;
    }

    /// Store a new reference value. Called by the method dispatcher
    /// on an `NV406E_SET_REFERENCE` method.
    #[inline]
    pub fn set_reference(&mut self, value: u32) {
        self.current_reference = value;
    }

    /// FNV-1a hash of the cursor's three fields prefixed with
    /// [`STATE_HASH_FORMAT_VERSION`], each field little-endian u32.
    ///
    /// Folds into the runtime's committed memory-state hash alongside
    /// [`cellgov_sync::ReservationTable::state_hash`]. The leading
    /// version byte lets the golden tests localise a shape change to
    /// this struct rather than letting it surface as a "memory state
    /// differs" divergence trace far from the actual cause.
    ///
    /// Hasher provenance: [`cellgov_mem::Fnv1aHasher`] uses the
    /// canonical 64-bit FNV-1a constants (offset basis
    /// `0xcbf29ce484222325`, prime `0x100000001b3`). If a future
    /// refactor changes those constants, the golden tests below
    /// fire immediately -- but the diagnostic will point at this
    /// file. Cross-reference `cellgov_mem::hash` if the golden
    /// changes without a shape change here.
    pub fn state_hash(&self) -> u64 {
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&[STATE_HASH_FORMAT_VERSION]);
        hasher.write(&self.put.to_le_bytes());
        hasher.write(&self.get.to_le_bytes());
        hasher.write(&self.current_reference.to_le_bytes());
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cursor_is_empty() {
        let cur = RsxFifoCursor::new();
        assert_eq!(cur.put(), 0);
        assert_eq!(cur.get(), 0);
        assert_eq!(cur.current_reference(), 0);
    }

    #[test]
    fn set_put_stores_value_verbatim() {
        let mut cur = RsxFifoCursor::new();
        cur.set_put(0x1000);
        assert_eq!(cur.put(), 0x1000);
        cur.set_put(0xDEAD_BEEF);
        assert_eq!(cur.put(), 0xDEAD_BEEF);
    }

    #[test]
    fn set_get_stores_value_verbatim() {
        let mut cur = RsxFifoCursor::new();
        cur.set_put(0x1000);
        cur.set_get(0x400);
        assert_eq!(cur.get(), 0x400);
        cur.set_get(0x1000);
        assert_eq!(cur.get(), 0x1000);
    }

    #[test]
    fn set_get_accepts_value_past_put_without_assertion() {
        // A legitimate ring wrap on a ring-sized FIFO can produce
        // new_get < prior_get and also new_get > put (temporarily,
        // mid-wrap). The cursor does NOT enforce get <= put because
        // it has no way to know the ring size. If the advance pass
        // writes a bogus value, the assertion surfaces there, not
        // here. This test pins the permissiveness as an intentional
        // contract.
        let mut cur = RsxFifoCursor::new();
        cur.set_put(0x100);
        cur.set_get(0x1_0000); // > put, no panic, no debug_assert
        assert_eq!(cur.get(), 0x1_0000);
        assert_eq!(cur.put(), 0x100);
    }

    #[test]
    fn backward_set_put_does_not_auto_reset_get() {
        // A guest that writes put = 0 after put = 0x2000 (a ring
        // reset) does NOT implicitly reset get. The advance pass
        // detects the "put went backward" case and issues its own
        // set_get(0) when appropriate. The cursor stays dumb.
        let mut cur = RsxFifoCursor::new();
        cur.set_put(0x2000);
        cur.set_get(0x1000);
        cur.set_put(0); // "reset" to 0
        assert_eq!(cur.put(), 0);
        assert_eq!(cur.get(), 0x1000, "get survives backward set_put");
    }

    #[test]
    fn set_reference_updates_independent_field() {
        let mut cur = RsxFifoCursor::new();
        cur.set_put(0x2000);
        cur.set_get(0x1000);
        cur.set_reference(0xDEAD_BEEF);
        assert_eq!(cur.current_reference(), 0xDEAD_BEEF);
        assert_eq!(cur.put(), 0x2000);
        assert_eq!(cur.get(), 0x1000);
    }

    #[test]
    fn reference_zero_is_indistinguishable_from_pristine() {
        // Documented behaviour: on real PS3 the initial reference is
        // zero, so a poll for reference=0 succeeds immediately, and
        // a game that writes NV406E_SET_REFERENCE with value 0 is
        // indistinguishable from one that never wrote. Pin the
        // semantics explicitly so a future refactor does not
        // "helpfully" add a has_reference_been_set flag that
        // diverges from hardware.
        let pristine = RsxFifoCursor::new();
        let mut set_to_zero = RsxFifoCursor::new();
        set_to_zero.set_reference(0);
        assert_eq!(pristine, set_to_zero);
        assert_eq!(pristine.state_hash(), set_to_zero.state_hash());
    }

    #[test]
    fn empty_cursor_hash_is_stable() {
        let a = RsxFifoCursor::new();
        let b = RsxFifoCursor::new();
        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_deterministic_across_identical_cursors() {
        let mut a = RsxFifoCursor::new();
        a.set_put(0xABCD);
        a.set_get(0x100);
        a.set_reference(0xFEEDFACE);

        let mut b = RsxFifoCursor::new();
        b.set_put(0xABCD);
        b.set_get(0x100);
        b.set_reference(0xFEEDFACE);

        assert_eq!(a.state_hash(), b.state_hash());
    }

    #[test]
    fn state_hash_distinguishes_each_field() {
        let mut base = RsxFifoCursor::new();
        base.set_put(1);

        let mut put_different = RsxFifoCursor::new();
        put_different.set_put(2);
        assert_ne!(
            put_different.state_hash(),
            base.state_hash(),
            "put distinguishes"
        );

        let mut get_different = RsxFifoCursor::new();
        get_different.set_put(1);
        get_different.set_get(1);
        assert_ne!(
            get_different.state_hash(),
            base.state_hash(),
            "get distinguishes"
        );

        let mut ref_different = RsxFifoCursor::new();
        ref_different.set_put(1);
        ref_different.set_reference(1);
        assert_ne!(
            ref_different.state_hash(),
            base.state_hash(),
            "reference distinguishes"
        );
    }

    #[test]
    fn state_hash_distinguishes_raw_put_from_masked_equivalent() {
        // Two cursors with observably-different put values that
        // happen to produce the same "effective FIFO address" after
        // caller-side masking must NOT collapse into the same state
        // hash. This is the "guest reads back what it wrote"
        // principle: the hash captures the raw stored value because
        // the guest observes the raw stored value.
        let mut raw = RsxFifoCursor::new();
        raw.set_put(0x7FFF_FFFF);
        let mut masked = RsxFifoCursor::new();
        masked.set_put(0x7FFF_FFFF & 0xFFFF);
        assert_ne!(raw.state_hash(), masked.state_hash());
    }

    #[test]
    fn empty_cursor_hash_golden() {
        // Pin the FNV-1a hash of [version_byte, 12 zero bytes]. A
        // change to STATE_HASH_FORMAT_VERSION, the field order, the
        // field widths, or the hasher family breaks this. If this
        // golden fires, check the version constant and update this
        // expected value in the same commit that justified the shape
        // change.
        //
        // Note: an empty cursor happens to be symmetric under
        // field-order swaps (all three fields are zero), so this
        // golden alone does NOT catch an accidental reorder of the
        // put / get / reference writes. `populated_cursor_hash_golden`
        // below provides that coverage.
        const EXPECTED: u64 = 0xeca4_bd25_1670_946c;
        let actual = RsxFifoCursor::new().state_hash();
        assert_eq!(
            actual, EXPECTED,
            "empty cursor hash drift: got 0x{:016x}, expected 0x{:016x}",
            actual, EXPECTED
        );
    }

    #[test]
    fn populated_cursor_hash_golden() {
        // Pin a non-trivial cursor with distinct values per field:
        // put=1, get=2, reference=3. This fails loudly with a
        // non-empty diagnostic if the hasher.write ordering in
        // state_hash is accidentally reordered -- the empty-cursor
        // golden cannot catch that because all three fields are
        // zero there.
        //
        // If this golden fires while the empty-cursor golden does
        // not, the problem is field-order; if both fire, the
        // problem is format-version / hasher / width.
        const EXPECTED: u64 = 0x3fed_cabe_847c_2bac;
        let mut cur = RsxFifoCursor::new();
        cur.set_put(1);
        cur.set_get(2);
        cur.set_reference(3);
        let actual = cur.state_hash();
        assert_eq!(
            actual, EXPECTED,
            "populated cursor hash drift: got 0x{:016x}, expected 0x{:016x}",
            actual, EXPECTED
        );
    }
}
