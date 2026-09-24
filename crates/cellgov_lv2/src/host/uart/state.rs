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

    /// True until `sys_uart_initialize`; the state hash skips a
    /// pristine UART.
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

    /// FNV-1a over every field via raw little-endian bytes per the
    /// host state-hash contract.
    pub(crate) fn state_hash(&self) -> u64 {
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
        let mut hasher = cellgov_mem::Fnv1aHasher::new();
        hasher.write(&[u8::from(*initialized)]);
        hasher.write(&(rx.len() as u64).to_le_bytes());
        hasher.write(rx);
        hasher.write(&(readers.len() as u64).to_le_bytes());
        for r in readers {
            hasher.write(&r.thread.raw().to_le_bytes());
            hasher.write(&r.buf_ptr.to_le_bytes());
            hasher.write(&r.size.to_le_bytes());
        }
        hasher.write(&av_cmd_ver.to_le_bytes());
        hasher.write(&hdmi_events.to_le_bytes());
        hasher.write(&[*hdmi_behavior, u8::from(*head_b_initialized)]);
        hasher.write(&[u8::from(hdmi_res_set[0]), u8::from(hdmi_res_set[1])]);
        hasher.write(&[u8::from(hdcp_first_auth[0]), u8::from(hdcp_first_auth[1])]);
        hasher.write(&[*hdmi_to_state]);
        hasher.finish()
    }
}
