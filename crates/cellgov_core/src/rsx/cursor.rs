//! Put / get / reference triple backing the RSX FIFO.
//!
//! Pure-data committed state for the cursor; ring-buffer geometry
//! and drain conditions live in the advance pass.

use cellgov_mem::lanes::{self, source, LaneValue, ObjectLanes};

/// Put / get / reference triple backing the RSX FIFO.
///
/// Invariants (enforced by the advance pass, not here):
///
/// - `put` is only written from the guest-side IO writeback path.
/// - `get` is only written from the advance pass (or savestate
///   restore).
/// - `get <= put` modulo the ring size known to the advance pass.
///
/// Field mutators have no cross-field side effects; the sync-state
/// lanes carry raw stored values.
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

    /// The cursor's term of the sync-state sum, computed on read.
    pub fn sync_term(&self) -> u128 {
        lanes::value_term(source::RSX_CURSOR, 0, self)
    }
}

/// One lane field per stored value:
///
/// 1. `put`
/// 2. `get`
/// 3. `current_reference`
impl LaneValue for RsxFifoCursor {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.put));
        lanes.lane(2, 0, u64::from(self.get));
        lanes.lane(3, 0, u64::from(self.current_reference));
    }
}

#[cfg(test)]
#[path = "tests/cursor_tests.rs"]
mod tests;
