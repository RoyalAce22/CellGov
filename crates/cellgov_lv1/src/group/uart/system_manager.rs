//! The answers to `write_virtual_uart` calls that write bytes to the
//! system manager's port.
//!
//! The system manager's receive function reads a header and the payload
//! bytes that the header counts, and only then runs a request handler;
//! see [`cellgov_ps3_abi::lv1::uart`]. CellGov answers these writes:
//! - a write that ends inside the payload of a request;
//! - a write that completes a request whose header counts 0 payload
//!   bytes. The receive function runs no handler for it.
//!
//! No source gives the reply of a request handler. So a write that
//! completes a request with a handler is a refusal that names the
//! request's service id.

use cellgov_ps3_abi::lv1::uart::{
    system_manager_header_payload_bytes, system_manager_header_service,
    system_manager_payload_read, GameOsVirtualUartPort, SYSTEM_MANAGER_HEADER_BYTES,
};

use super::request::{PendingRequest, REQUEST_PREFIX_BYTES};
use super::write::{after_status_step, refuse, written_answer, SentRequest, WriteInput};
use super::UartChange;
use crate::answer::{HypercallDisposition, HypercallRefusal};
use crate::request::HypercallRequest;

const HEADER: usize = SYSTEM_MANAGER_HEADER_BYTES as usize;

// The kept prefix holds the whole header, so `progress` slices it in bounds.
const _: () = assert!(HEADER <= REQUEST_PREFIX_BYTES);

/// The refusal of a write that ends inside a request header.
///
/// The receive function reads the header with one read, and ends with no
/// handler when that read gives fewer bytes
/// [OFW-4.93 lv1 a:0x44CDB0-0x44CDB4]. No image read gives when the
/// system manager reads, so the read can take the first bytes alone.
pub(super) const HEADER_SPLIT_REFUSAL: &str =
    "a virtual UART write that ends inside a request header on the system manager's port, whose \
     one header read can take the first bytes alone and drop them";

/// The refusal of a write whose bytes complete a request and are more
/// than its bytes.
pub(super) const OVERRUN_REFUSAL: &str =
    "a virtual UART write whose bytes complete a request on the system manager's port and start \
     a second request";

/// The state of a system manager request after a write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Progress {
    /// The partition did not send each byte of the header.
    InHeader,
    /// The partition sent the header, and not each payload byte that the
    /// receive function reads.
    InPayload,
    /// The partition sent each byte that the receive function reads, and
    /// `extra` bytes more.
    Complete {
        /// The request header.
        header: [u8; HEADER],
        /// The bytes past the end of the request.
        extra: u64,
    },
}

/// The state of the request `request` on the system manager's port.
///
/// The receive function reads 16 header bytes and the payload bytes that
/// [`system_manager_payload_read`] gives. A header that counts 0 payload
/// bytes completes the request with its last byte.
fn progress(request: &PendingRequest) -> Progress {
    if request.sent < SYSTEM_MANAGER_HEADER_BYTES {
        return Progress::InHeader;
    }
    let mut header = [0; HEADER];
    header.copy_from_slice(&request.prefix[..HEADER]);
    let payload = system_manager_payload_read(system_manager_header_payload_bytes(&header));
    let bytes = SYSTEM_MANAGER_HEADER_BYTES + u64::from(payload.unwrap_or(0));
    if request.sent < bytes {
        return Progress::InPayload;
    }
    Progress::Complete {
        header,
        extra: request.sent - bytes,
    }
}

/// The answer to a write of `sent` to the system manager's port `row`.
///
/// The answer is r3 = 0 with the bytes written in r4. It carries the
/// state of the port's request after the write:
/// - for a write that ends inside the payload, the request's bytes;
/// - for a write that completes a request whose header counts 0 payload
///   bytes, no request. The receive function runs no handler and writes
///   no reply [OFW-4.93 lv1 a:0x44CDD0-0x44CDD8].
///
/// The write leaves the port's receive buffer as it is, so the status
/// step after the write changes no bit.
///
/// The call is a refusal in these cases:
/// - the write ends inside the header; see [`HEADER_SPLIT_REFUSAL`];
/// - the write completes a request and holds bytes past its end;
/// - the write completes a request for which the receive function runs a
///   handler. The refusal names the header's service id;
/// - the status step refuses.
pub(super) fn write(
    request: &HypercallRequest,
    input: &WriteInput<'_>,
    firmware: &str,
    row: &GameOsVirtualUartPort,
    sent: &SentRequest,
) -> HypercallDisposition {
    let next = sent.pending.with(&sent.bytes);
    let left = match progress(&next) {
        Progress::InHeader => return refuse(request, HEADER_SPLIT_REFUSAL),
        Progress::InPayload => next,
        Progress::Complete { extra, .. } if extra > 0 => {
            return refuse(request, OVERRUN_REFUSAL);
        }
        Progress::Complete { header, .. } => {
            if system_manager_payload_read(system_manager_header_payload_bytes(&header)).is_some() {
                return HypercallDisposition::Refused(HypercallRefusal::SystemManagerRequest {
                    unit: request.unit,
                    pc: request.pc,
                    service: system_manager_header_service(&header),
                });
            }
            PendingRequest::empty(row.port)
        }
    };
    let mut answer = written_answer(sent.bytes.len() as u64);
    answer.push_change(UartChange::RequestBytesSent(left));
    let received = input.uart.received(row.port).len() as u64;
    after_status_step(request, input, firmware, row, answer, received)
}

#[cfg(test)]
#[path = "tests/system_manager_tests.rs"]
mod tests;
