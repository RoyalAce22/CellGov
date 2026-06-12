//! Put / get / reference triple backing the RSX FIFO.
//!
//! Pure-data committed state for the cursor; ring-buffer geometry
//! and drain conditions live in the advance pass.

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
/// Field mutators have no cross-field side effects; the state hash
/// captures raw stored values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RsxFifoCursor {
    put: u32,
    get: u32,
    current_reference: u32,
}

impl RsxFifoCursor {
    /// Pristine cursor with all fields zero.
    #[inline]
    pub const fn new() -> Self {
        Self {
            put: 0,
            get: 0,
            current_reference: 0,
        }
    }

    /// Current put pointer (mirror of [`super::control_register::PUT_ADDR`]).
    #[inline]
    pub const fn put(self) -> u32 {
        self.put
    }

    /// Current get pointer (mirror of [`super::control_register::GET_ADDR`]).
    #[inline]
    pub const fn get(self) -> u32 {
        self.get
    }

    /// Current reference value (mirror of [`super::control_register::REF_ADDR`]).
    #[inline]
    pub const fn current_reference(self) -> u32 {
        self.current_reference
    }

    /// Store a new put value verbatim.
    #[inline]
    pub fn set_put(&mut self, put: u32) {
        self.put = put;
    }

    /// Store a new get value verbatim.
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
#[path = "tests/cursor_tests.rs"]
mod tests;
