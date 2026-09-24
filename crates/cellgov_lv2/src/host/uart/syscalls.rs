//! The four `sys_uart` syscalls and the send-side parse, commit, and delivery.

use cellgov_effects::{Effect, WritePayload};
use cellgov_event::UnitId;
use cellgov_mem::ByteRange;
use cellgov_ps3_abi::lv2::errno;
use cellgov_ps3_abi::lv2::uart as av;
use cellgov_time::GuestTicks;

use crate::dispatch::{Lv2BlockReason, Lv2Dispatch, PendingResponse};
use crate::host::{Lv2Host, Lv2Runtime};

use super::cid_table::{cid_spec, SizeRule};
use super::packet::{padded, rd16, rd32};
use super::reply::ReplyBatch;
use super::state::UartReader;

impl Lv2Host {
    /// `sys_uart_initialize` (367).
    ///
    /// # Errors
    ///
    /// - `CELL_ENOSYS` for a caller without root privilege.
    /// - `CELL_EPERM` once the UART is already claimed.
    pub(in crate::host) fn dispatch_uart_initialize(&mut self) -> Lv2Dispatch {
        if !self.has_root_perm() {
            return Lv2Dispatch::immediate(errno::CELL_ENOSYS.into());
        }
        if self.state.uart.initialized {
            return Lv2Dispatch::immediate(errno::CELL_EPERM.into());
        }
        self.state.uart.initialized = true;
        Lv2Dispatch::immediate(0)
    }

    /// `sys_uart_get_params` (370): the two ring sizes, u64 BE each.
    ///
    /// # Errors
    ///
    /// - `CELL_ENOSYS` without root privilege.
    /// - `CELL_ESRCH` before `sys_uart_initialize`.
    /// - `CELL_EFAULT` for an unwritable output block.
    pub(in crate::host) fn dispatch_uart_get_params(
        &mut self,
        params_ptr: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if !self.has_root_perm() {
            return Lv2Dispatch::immediate(errno::CELL_ENOSYS.into());
        }
        if !self.state.uart.initialized {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        if params_ptr == 0 || !rt.writable(u64::from(params_ptr), av::SYS_UART_PARAMS_LEN) {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let mut out = [0u8; av::SYS_UART_PARAMS_LEN];
        out[..8].copy_from_slice(&(av::PS3AV_RX_BUF_SIZE as u64).to_be_bytes());
        out[8..].copy_from_slice(&(av::PS3AV_TX_BUF_SIZE as u64).to_be_bytes());
        Lv2Dispatch::Immediate {
            code: 0,
            effects: vec![Effect::shared_write(
                ByteRange::contiguous_u32(params_ptr, av::SYS_UART_PARAMS_LEN as u32),
                WritePayload::from_slice(&out),
                requester,
                tick,
            )],
        }
    }

    /// `sys_uart_receive` (368): pops up to `size` bytes of the reply
    /// stream into `buf_ptr` and returns the count. An empty stream
    /// returns 0 in non-blocking mode and parks a blocking caller
    /// behind any readers already parked, until sends stage enough
    /// bytes to reach it. A process exit purges the readers its
    /// threads parked.
    ///
    /// The kernel's non-blocking arm answers `CELL_EBUSY` only while
    /// another reader holds the receive lock mid-copy; a dispatch
    /// here is atomic, so that window does not exist and no arm
    /// returns `CELL_EBUSY`.
    ///
    /// # Errors
    ///
    /// - `CELL_ENOSYS` without root privilege.
    /// - `CELL_EINVAL` for a mode other than 0 / 1, or a transfer over
    ///   [`av::SYS_UART_MAX_TRANSFER`], which draws a named break.
    /// - `CELL_ESRCH` before `sys_uart_initialize`, or for a caller
    ///   with no PPU thread record.
    /// - `CELL_EFAULT` for an unwritable buffer, checked before any
    ///   byte leaves the stream.
    pub(in crate::host) fn dispatch_uart_receive(
        &mut self,
        buf_ptr: u32,
        size: u64,
        mode: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if !self.has_root_perm() {
            return Lv2Dispatch::immediate(errno::CELL_ENOSYS.into());
        }
        if size == 0 {
            return Lv2Dispatch::immediate(0);
        }
        if mode & !(av::SYS_UART_MODE_BLOCKING_BIG_OP | av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP) != 0
        {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if !self.state.uart.initialized {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        if size > av::SYS_UART_MAX_TRANSFER {
            self.log_invariant_break(
                "dispatch.uart_transfer_over_cap",
                format_args!(
                    "sys_uart_receive: {size} bytes exceeds the {} byte transfer cap; returning CELL_EINVAL",
                    av::SYS_UART_MAX_TRANSFER
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let available = self.state.uart.rx.len();
        if available == 0 {
            if mode != av::SYS_UART_MODE_BLOCKING_BIG_OP {
                return Lv2Dispatch::immediate(0);
            }
            let Some(thread) = self.state.ppu_threads.thread_id_for_unit(requester) else {
                return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
            };
            if !rt.writable(
                u64::from(buf_ptr),
                size.min(av::PS3AV_RX_BUF_SIZE as u64) as usize,
            ) {
                return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
            }
            if self.state.uart.readers.iter().any(|r| r.thread == thread) {
                // A parked thread cannot dispatch; two records for
                // one thread would wake it twice.
                self.log_invariant_break(
                    "dispatch.uart_reader_reparked",
                    format_args!(
                        "sys_uart_receive: {thread:?} is already parked on the reply stream; \
                         returning CELL_ESRCH"
                    ),
                );
                return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
            }
            if !self.state.uart.readers.is_empty() {
                self.obs.uart_readers_queued += 1;
            }
            self.state.uart.readers.push_back(UartReader {
                thread,
                buf_ptr,
                size,
            });
            return Lv2Dispatch::Block {
                reason: Lv2BlockReason::Uart,
                pending: PendingResponse::ReturnCode { code: 0 },
                effects: vec![],
            };
        }
        let n = (size as usize).min(available);
        if !rt.writable(u64::from(buf_ptr), n) {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        }
        let bytes: Vec<u8> = self.state.uart.rx.drain(..n).collect();
        Lv2Dispatch::Immediate {
            code: n as u64,
            effects: vec![Effect::shared_write(
                ByteRange::contiguous_u32(buf_ptr, n as u32),
                WritePayload::from_slice(&bytes),
                requester,
                tick,
            )],
        }
    }

    /// `sys_uart_send` (369): parses every packet in the buffer,
    /// stages the replies and any events they trigger, and hands the
    /// stream to the parked readers in park order, each taking up to
    /// its own size while bytes remain. The walk advances by each
    /// header's u16 length plus the four bytes before it, in 16-bit
    /// arithmetic. An `AVB_PARAM` shorter than its own sub-packet
    /// counts is a size mismatch. A mode-0 send larger than the TX
    /// ring reports its first chunk, [`av::SYS_UART_CHUNK`] bytes at
    /// most.
    ///
    /// # Errors
    ///
    /// - `CELL_ENOSYS` without root privilege.
    /// - `CELL_EINVAL` for a mode above 3, or a transfer over
    ///   [`av::SYS_UART_MAX_TRANSFER`], which draws a named break.
    /// - `CELL_ESRCH` before `sys_uart_initialize`.
    /// - `CELL_EFAULT` for an unreadable buffer.
    /// - `CELL_EAGAIN` in mode 2 when the buffer exceeds the TX ring.
    pub(in crate::host) fn dispatch_uart_send(
        &mut self,
        buf_ptr: u32,
        size: u64,
        mode: u32,
        requester: UnitId,
        rt: &dyn Lv2Runtime,
        tick: GuestTicks,
    ) -> Lv2Dispatch {
        if !self.has_root_perm() {
            return Lv2Dispatch::immediate(errno::CELL_ENOSYS.into());
        }
        if size == 0 {
            return Lv2Dispatch::immediate(0);
        }
        if mode
            & !(av::SYS_UART_MODE_BLOCKING_BIG_OP
                | av::SYS_UART_MODE_NOT_BLOCKING_OP
                | av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP)
            != 0
        {
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        if !self.state.uart.initialized {
            return Lv2Dispatch::immediate(errno::CELL_ESRCH.into());
        }
        if size > av::SYS_UART_MAX_TRANSFER {
            self.log_invariant_break(
                "dispatch.uart_transfer_over_cap",
                format_args!(
                    "sys_uart_send: {size} bytes exceeds the {} byte transfer cap; returning CELL_EINVAL",
                    av::SYS_UART_MAX_TRANSFER
                ),
            );
            return Lv2Dispatch::immediate(errno::CELL_EINVAL.into());
        }
        let Some(tx) = rt.read_committed(u64::from(buf_ptr), size as usize) else {
            return Lv2Dispatch::immediate(errno::CELL_EFAULT.into());
        };
        if mode == av::SYS_UART_MODE_NOT_BLOCKING_OP && size > av::PS3AV_TX_BUF_SIZE as u64 {
            return Lv2Dispatch::immediate(errno::CELL_EAGAIN.into());
        }
        // Mode 0 pushes its first chunk and, when the ring cannot take
        // that chunk whole, reports the chunk's size rather than the
        // ring's; the bytes past the ring are dropped either way. No
        // firmware caller reaches this arm -- vsh.self sends in mode 2
        // only -- so the count the kernel really returns is
        // unestablished.
        let sent = if mode == av::SYS_UART_MODE_NOT_BLOCKING_BIG_OP
            && size > av::PS3AV_TX_BUF_SIZE as u64
        {
            size.min(av::SYS_UART_CHUNK)
        } else {
            size
        };
        let tx = tx.to_vec();
        let mut batch = ReplyBatch::default();
        self.uart_parse(&tx, &mut batch);
        self.uart_commit(batch);
        let deliveries = self.uart_deliver_to_readers(requester, tick);
        if deliveries.is_empty() {
            return Lv2Dispatch::immediate(sent);
        }
        let mut woken_unit_ids = Vec::with_capacity(deliveries.len());
        let mut response_updates = Vec::with_capacity(deliveries.len());
        let mut effects = Vec::with_capacity(deliveries.len());
        for (unit, effect, code) in deliveries {
            woken_unit_ids.push(unit);
            response_updates.push((unit, PendingResponse::ReturnCode { code }));
            effects.push(effect);
        }
        Lv2Dispatch::WakeAndReturn {
            code: sent,
            woken_unit_ids,
            response_updates,
            effects,
        }
    }

    /// Append a batch to the reply stream: plain replies, then
    /// system-controller replies, then events.
    fn uart_commit(&mut self, batch: ReplyBatch) {
        self.obs.uart_rx_overflow_bytes += batch.dropped;
        for chunk in [batch.plain, batch.syscon, batch.events] {
            let room = av::PS3AV_RX_BUF_SIZE.saturating_sub(self.state.uart.rx.len());
            let take = chunk.len().min(room);
            self.obs.uart_rx_overflow_bytes += (chunk.len() - take) as u64;
            self.state.uart.rx.extend_from_slice(&chunk[..take]);
        }
    }

    /// Hand the stream to the parked readers, front first, while
    /// bytes remain: one `(unit, write, count)` per reader served.
    /// A reader whose thread record is gone is dropped from the
    /// queue (named break) without consuming bytes.
    fn uart_deliver_to_readers(
        &mut self,
        requester: UnitId,
        tick: GuestTicks,
    ) -> Vec<(UnitId, Effect, u64)> {
        let mut served = Vec::new();
        while !self.state.uart.rx.is_empty() {
            let Some(reader) = self.state.uart.readers.pop_front() else {
                break;
            };
            let Some(unit) = self.resolve_wake_thread(reader.thread, "uart_deliver.reader") else {
                continue;
            };
            let n = (reader.size as usize).min(self.state.uart.rx.len());
            let bytes: Vec<u8> = self.state.uart.rx.drain(..n).collect();
            served.push((
                unit,
                Effect::shared_write(
                    ByteRange::contiguous_u32(reader.buf_ptr, n as u32),
                    WritePayload::from_slice(&bytes),
                    requester,
                    tick,
                ),
                n as u64,
            ));
        }
        served
    }

    /// Walk the packets in one send.
    fn uart_parse(&mut self, tx: &[u8], batch: &mut ReplyBatch) {
        if tx.len() >= av::PS3AV_TX_BUF_SIZE {
            // An overfull ring draws one overflow reply, addressed
            // by the low half of the first cid.
            let cid = u32::from(rd16(&padded(tx, 0, 8), 6));
            batch.reply(false, cid, av::PS3AV_STATUS_BUFFER_OVERFLOW, &[]);
            return;
        }
        let mut off = 0;
        while off < tx.len() {
            let hdr = padded(tx, off, av::PS3AV_HEADER_LEN);
            let version = rd16(&hdr, 0);
            let length = rd16(&hdr, 2);
            let cid = rd32(&hdr, 4);
            // The AV manager sizes a packet in 16-bit arithmetic and
            // walks by that size however small it is; the poison
            // length is the one value that would walk zero bytes.
            let pkt_size = usize::from(length.wrapping_add(4));
            if length == av::PS3AV_LENGTH_POISON {
                batch.reply(
                    false,
                    av::PS3AV_CID_POISON_REPLY,
                    av::PS3AV_STATUS_FAILURE,
                    &[],
                );
                return;
            }
            if version != av::PS3AV_VERSION {
                batch.reply(false, cid & 0xFFFF, av::PS3AV_STATUS_INVALID_COMMAND, &[]);
                return;
            }
            // The handler's view always covers a header; the walk
            // does not.
            let pkt = padded(tx, off, pkt_size.max(av::PS3AV_HEADER_LEN));
            off += pkt_size;
            *self.obs.uart_cids.entry(cid).or_insert(0) += 1;
            let Some(spec) = cid_spec(cid) else {
                *self.obs.uart_unknown_cids.entry(cid).or_insert(0) += 1;
                self.log_invariant_break(
                    "dispatch.uart_unknown_cid",
                    format_args!(
                        "sys_uart_send: no AV-manager handler for cid 0x{cid:08x}; the guest gets no reply"
                    ),
                );
                continue;
            };
            let expected = match spec.size {
                SizeRule::Exact(n) => Some(n),
                SizeRule::Unchecked => None,
                SizeRule::Computed(f) => Some(f(&pkt)),
            };
            if let Some(expected) = expected {
                if expected != pkt_size {
                    batch.reply(
                        false,
                        cid & 0xFFFF,
                        av::PS3AV_STATUS_INVALID_SAMPLE_SIZE,
                        &[],
                    );
                    return;
                }
            }
            (spec.run)(self, cid, &pkt, batch);
        }
    }
}
