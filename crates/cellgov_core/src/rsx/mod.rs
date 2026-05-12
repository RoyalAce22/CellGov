//! RSX CPU-side completion state.
//!
//! Owns the pure-data committed state for the FIFO cursor and the
//! submodules covering methods, flip, and reports. Ring-buffer
//! geometry and drain conditions live in the advance pass; this
//! module is too narrow to see them.

pub mod advance;
pub mod flip;
pub mod method;
pub mod reports;

/// Guest address of the RSX control register's `put` slot.
pub const RSX_CONTROL_PUT_ADDR: u32 = 0xC000_0040;

/// Guest address of the RSX control register's `get` slot.
pub const RSX_CONTROL_GET_ADDR: u32 = 0xC000_0044;

/// Guest address of the RSX control register's `reference` slot.
pub const RSX_CONTROL_REF_ADDR: u32 = 0xC000_0048;

/// Guest address of the fixed-address flip-status mirror.
///
/// Written as a 4-byte big-endian u32 with the status in the low
/// byte. Updated only on transitions, so a reader observes each
/// status change exactly once.
pub const RSX_FLIP_STATUS_MIRROR_ADDR: u32 = 0xC000_0050;

/// Hash-input shape version. Bump when [`RsxFifoCursor::state_hash`]
/// changes field order, count, endianness, or hasher family.
pub const STATE_HASH_FORMAT_VERSION: u8 = 1;

/// Put / get / reference triple backing the RSX FIFO.
///
/// Invariants (enforced by the advance pass, not here):
///
/// - `put` is only written from the guest-side IO writeback path.
/// - `get` is only written from the advance pass (or savestate
///   restore).
/// - `get <= put` modulo the ring size known to the advance pass.
///
/// Mutators accept any `u32`; the state hash captures raw stored
/// values so that two observably-different guest writes never
/// collapse into the same committed state. Field mutators have no
/// cross-field side effects, which is what makes savestate restore
/// order-independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RsxFifoCursor {
    put: u32,
    get: u32,
    current_reference: u32,
}

impl RsxFifoCursor {
    /// Pristine cursor: put, get, and reference all zero.
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
    #[inline]
    pub const fn current_reference(self) -> u32 {
        self.current_reference
    }

    /// Store a new put value verbatim.
    ///
    /// Legitimate caller: the RSX IO region writeback path.
    #[inline]
    pub fn set_put(&mut self, put: u32) {
        self.put = put;
    }

    /// Store a new get value verbatim.
    ///
    /// Legitimate callers: the FIFO advance pass and savestate
    /// restore. Other callers break the "get is only
    /// advance-written" invariant.
    #[inline]
    pub fn set_get(&mut self, get: u32) {
        self.get = get;
    }

    /// Store a new reference value.
    #[inline]
    pub fn set_reference(&mut self, value: u32) {
        self.current_reference = value;
    }

    /// FNV-1a hash prefixed with [`STATE_HASH_FORMAT_VERSION`], each
    /// field little-endian u32. Folds into the runtime's committed
    /// memory-state hash.
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
        let mut cur = RsxFifoCursor::new();
        cur.set_put(0x100);
        cur.set_get(0x1_0000);
        assert_eq!(cur.get(), 0x1_0000);
        assert_eq!(cur.put(), 0x100);
    }

    #[test]
    fn backward_set_put_does_not_auto_reset_get() {
        let mut cur = RsxFifoCursor::new();
        cur.set_put(0x2000);
        cur.set_get(0x1000);
        cur.set_put(0);
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
        let mut raw = RsxFifoCursor::new();
        raw.set_put(0x7FFF_FFFF);
        let mut masked = RsxFifoCursor::new();
        masked.set_put(0x7FFF_FFFF & 0xFFFF);
        assert_ne!(raw.state_hash(), masked.state_hash());
    }

    #[test]
    fn empty_cursor_hash_golden() {
        // All-zero fields are symmetric under field-order swaps;
        // `populated_cursor_hash_golden` covers reorder.
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
