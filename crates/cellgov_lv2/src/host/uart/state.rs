//! The UART state: the reply stream, the parked readers, and the AV manager's HDMI state.

use std::collections::VecDeque;

use cellgov_ps3_abi::lv2::uart as av;

use crate::ppu_thread::PpuThreadId;

use super::hdmi::HDMI_STATE_PLUGGED;

/// A blocking reader parked on an empty reply stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UartReader {
    pub thread: PpuThreadId,
    pub buf_ptr: u32,
    pub size: u64,
}

/// Reply stream, parked readers, and the AV manager's HDMI state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UartState {
    pub(super) initialized: bool,
    /// Undelivered reply and event bytes, oldest first.
    pub(super) rx: Vec<u8>,
    /// Blocking readers in park order; the front takes the next
    /// bytes. Which of several parked readers wins a send is a race on
    /// a console, so park order here is CellGov's own.
    pub(super) readers: VecDeque<UartReader>,
    /// Version the last AV_INIT carried; events echo it.
    pub(super) av_cmd_ver: u16,
    /// Enabled-event mask (`PS3AV_EVENT_BIT_*`).
    pub(super) hdmi_events: u32,
    pub(super) hdmi_behavior: u8,
    pub(super) head_b_initialized: bool,
    pub(super) hdmi_res_set: [bool; 2],
    pub(super) hdcp_first_auth: [bool; 2],
    /// Last state the HDMI 0 event script was driven to; the next
    /// script starts one step below its own first state or here,
    /// whichever is lower.
    pub(super) hdmi_to_state: u8,
}

impl UartState {
    pub(crate) fn new() -> Self {
        Self {
            initialized: false,
            rx: Vec::new(),
            readers: VecDeque::new(),
            av_cmd_ver: 0,
            hdmi_events: 0,
            hdmi_behavior: av::PS3AV_HDMI_BEHAVIOR_NORMAL,
            head_b_initialized: false,
            hdmi_res_set: [false; 2],
            hdcp_first_auth: [true; 2],
            hdmi_to_state: HDMI_STATE_PLUGGED,
        }
    }

    /// True until `sys_uart_initialize`.
    #[cfg(test)]
    pub(crate) fn is_pristine(&self) -> bool {
        *self == Self::new()
    }

    #[cfg(test)]
    pub(crate) fn pending_bytes(&self) -> &[u8] {
        &self.rx
    }

    #[cfg(test)]
    pub(crate) fn readers(&self) -> &VecDeque<UartReader> {
        &self.readers
    }

    /// Remove every parked reader whose thread is in `threads`,
    /// preserving the order of survivors; returns the removed
    /// records. Process-exit purge: a reader of an exited process
    /// would otherwise be served first and its bytes dropped with
    /// the wake, ahead of a live reader behind it.
    #[must_use = "the purged readers are the only witness that these wakes were cancelled"]
    pub(crate) fn purge_readers_of(
        &mut self,
        threads: &std::collections::BTreeSet<PpuThreadId>,
    ) -> Vec<UartReader> {
        let mut removed = Vec::new();
        self.readers.retain(|r| {
            if threads.contains(&r.thread) {
                removed.push(*r);
                false
            } else {
                true
            }
        });
        removed
    }

    #[cfg(test)]
    pub(crate) fn hdmi_events(&self) -> u32 {
        self.hdmi_events
    }

    /// The UART's term of the sync-state sum, computed on read.
    pub(crate) fn sync_term(&self) -> u128 {
        cellgov_mem::lanes::value_term(cellgov_mem::lanes::source::UART, 0, self)
    }
}

/// Lane layout of the UART:
///
/// - Fields 1 to 3 are the initialized flag, the reply length and the
///   reply bytes.
/// - Field 4 is the reader count.
/// - Fields 5 to 7 are parked reader `i`, at slot `i`.
/// - Fields 8 to 14 are the HDMI state. Each two-port array has one
///   slot per port.
///
/// The exhaustive destructure makes a new field without a lane a
/// compile error.
impl cellgov_mem::lanes::LaneValue for UartState {
    fn lanes(&self, lanes: &mut cellgov_mem::lanes::ObjectLanes) {
        let Self {
            initialized,
            rx,
            readers,
            av_cmd_ver,
            hdmi_events,
            hdmi_behavior,
            head_b_initialized,
            hdmi_res_set,
            hdcp_first_auth,
            hdmi_to_state,
        } = self;
        lanes.lane(1, 0, u64::from(*initialized));
        lanes.lane(2, 0, rx.len() as u64);
        lanes.bytes(3, &[], rx);
        lanes.lane(4, 0, readers.len() as u64);
        for (slot, r) in readers.iter().enumerate() {
            let slot = slot as u64;
            lanes.lane(5, slot, r.thread.raw());
            lanes.lane(6, slot, u64::from(r.buf_ptr));
            lanes.lane(7, slot, r.size);
        }
        lanes.lane(8, 0, u64::from(*av_cmd_ver));
        lanes.lane(9, 0, u64::from(*hdmi_events));
        lanes.lane(10, 0, u64::from(*hdmi_behavior));
        lanes.lane(11, 0, u64::from(*head_b_initialized));
        for port in 0..2 {
            lanes.lane(12, port as u64, u64::from(hdmi_res_set[port]));
            lanes.lane(13, port as u64, u64::from(hdcp_first_auth[port]));
        }
        lanes.lane(14, 0, u64::from(*hdmi_to_state));
    }
}

#[cfg(test)]
#[path = "tests/uart_lanes_tests.rs"]
mod lanes_tests;
