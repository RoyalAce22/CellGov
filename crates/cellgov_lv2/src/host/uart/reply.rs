//! The reply staging buffers one send fills.

use cellgov_ps3_abi::lv2::uart as av;

/// Replies staged while one send's packets execute. The AV manager
/// keeps two staging buffers and commits the plain one before the
/// system-controller one; events go straight to the stream after
/// both. The overflow checks every command makes look at the plain
/// buffer only.
#[derive(Debug, Default)]
pub(super) struct ReplyBatch {
    pub(super) plain: Vec<u8>,
    pub(super) syscon: Vec<u8>,
    pub(super) events: Vec<u8>,
    /// Bytes a full staging buffer refused.
    pub(super) dropped: u64,
}

impl ReplyBatch {
    fn free(&self) -> usize {
        av::PS3AV_RX_BUF_SIZE.saturating_sub(self.plain.len())
    }

    pub(super) fn refuse_if_full(&mut self, cid: u32, body_len: usize) -> bool {
        if self.free() < av::PS3AV_REPLY_HEADER_LEN + body_len {
            self.reply(false, cid, av::PS3AV_STATUS_BUFFER_OVERFLOW, &[]);
            true
        } else {
            false
        }
    }

    /// Stage a reply header (`version, length, cid | REPLY, status`)
    /// and body into the plain or system-controller buffer.
    pub(super) fn reply(&mut self, syscon: bool, cid: u32, status: u32, body: &[u8]) {
        let total = av::PS3AV_REPLY_HEADER_LEN + body.len();
        let target = if syscon {
            &mut self.syscon
        } else {
            &mut self.plain
        };
        if target.len() + total > av::PS3AV_RX_BUF_SIZE {
            self.dropped += total as u64;
            return;
        }
        target.extend_from_slice(&av::PS3AV_VERSION.to_be_bytes());
        target.extend_from_slice(&((body.len() + 8) as u16).to_be_bytes());
        target.extend_from_slice(&(cid | av::PS3AV_REPLY_BIT).to_be_bytes());
        target.extend_from_slice(&status.to_be_bytes());
        target.extend_from_slice(body);
    }
}
