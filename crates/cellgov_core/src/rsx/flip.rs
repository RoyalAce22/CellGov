//! RSX flip-status state machine.
//!
//! Two terminal status values (`DONE = 0`, `WAITING = 1`) driven at
//! commit boundaries:
//!
//! - On `RsxFlipRequest` commit: `status = WAITING`, `pending = true`.
//! - On the next commit boundary: `status = DONE`, `pending = false`.
//!
//! Any PPU step between those two boundaries observes WAITING.
//! `handler` records the `cellGcmSetFlipHandler` address but PPU
//! dispatch into it is not modelled.

use cellgov_mem::lanes::{self, source, LaneValue, ObjectLanes};

pub use cellgov_ps3_abi::hw::rsx::{
    CELL_GCM_DISPLAY_FLIP_STATUS_DONE, CELL_GCM_DISPLAY_FLIP_STATUS_WAITING,
};

/// Guest address of the fixed-address flip-status mirror.
///
/// Written as a 4-byte big-endian u32 with the status in the low
/// byte. Updated only on transitions, so a reader observes each
/// status change exactly once. The writer is the commit pipeline,
/// not [`RsxFlipState`]; this constant lives here for semantic
/// ownership of the flip-status domain.
pub const RSX_FLIP_STATUS_MIRROR_ADDR: u32 = 0xC000_0050;

/// RSX flip-status state tracked across commit boundaries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RsxFlipState {
    /// Current status byte; starts at `DONE`.
    status: u8,
    /// Callback address from `cellGcmSetFlipHandler`. Zero if none.
    /// Recorded only; PPU dispatch into it is not modelled.
    handler: u32,
    /// True between an `RsxFlipRequest` commit and the next commit
    /// boundary's DONE transition.
    pending: bool,
    /// Back-buffer index from the most-recent flip request.
    /// Last-writer-wins when multiple requests arrive before the
    /// next commit boundary.
    buffer_index: u8,
}

impl RsxFlipState {
    /// Pristine state: `status = DONE`, no handler, nothing pending.
    #[inline]
    pub const fn new() -> Self {
        Self {
            status: CELL_GCM_DISPLAY_FLIP_STATUS_DONE,
            handler: 0,
            pending: false,
            buffer_index: 0,
        }
    }

    /// Current flip-status byte.
    #[inline]
    pub const fn status(&self) -> u8 {
        self.status
    }

    /// Registered flip-handler address; zero if none.
    #[inline]
    pub const fn handler(&self) -> u32 {
        self.handler
    }

    /// Whether a flip is pending the WAITING -> DONE transition.
    #[inline]
    pub const fn pending(&self) -> bool {
        self.pending
    }

    /// Back-buffer index from the most-recent flip request.
    /// Meaningful only while `pending()` is true.
    #[inline]
    pub const fn buffer_index(&self) -> u8 {
        self.buffer_index
    }

    /// Overwrite the full state.
    ///
    /// Legitimate callers: savestate restore and tests. Normal
    /// operation uses the typed mutators.
    #[inline]
    pub fn restore(&mut self, status: u8, handler: u32, pending: bool, buffer_index: u8) {
        self.status = status;
        self.handler = handler;
        self.pending = pending;
        self.buffer_index = buffer_index;
    }

    /// Record the flip-handler callback address.
    #[inline]
    pub fn set_handler(&mut self, addr: u32) {
        self.handler = addr;
    }

    /// Transition to WAITING on an `NV4097_FLIP_BUFFER` parse.
    /// A second request before completion overwrites `buffer_index`
    /// but keeps `pending` and `status` as WAITING.
    #[inline]
    pub fn request_flip(&mut self, buffer_index: u8) {
        self.status = CELL_GCM_DISPLAY_FLIP_STATUS_WAITING;
        self.pending = true;
        self.buffer_index = buffer_index;
    }

    /// Complete a pending flip at a commit boundary; no-op when
    /// `pending == false`. Must run one commit after the
    /// `RsxFlipRequest` so a PPU step can observe WAITING.
    /// The completion keeps `buffer_index`, so its sync-state lane
    /// does not move.
    #[inline]
    pub fn complete_pending_flip(&mut self) -> bool {
        if self.pending {
            self.status = CELL_GCM_DISPLAY_FLIP_STATUS_DONE;
            self.pending = false;
            true
        } else {
            false
        }
    }

    /// The flip state's term of the sync-state sum, computed on read.
    pub fn sync_term(&self) -> u128 {
        lanes::value_term(source::RSX_FLIP, 0, self)
    }
}

/// One lane field per stored value:
///
/// 1. `status`
/// 2. `handler`
/// 3. `pending`
/// 4. `buffer_index`
impl LaneValue for RsxFlipState {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, u64::from(self.status));
        lanes.lane(2, 0, u64::from(self.handler));
        lanes.lane(3, 0, u64::from(self.pending));
        lanes.lane(4, 0, u64::from(self.buffer_index));
    }
}

impl Default for RsxFlipState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
#[path = "tests/flip_tests.rs"]
mod tests;
