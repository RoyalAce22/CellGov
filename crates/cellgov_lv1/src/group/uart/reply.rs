//! The bytes that the dispatcher manager writes to its port for a
//! complete request: a reply header, then the service's reply bytes.
//!
//! The service writes its reply fields over the request bytes, so each
//! other byte of the reply is the request byte at the same offset; see
//! [`cellgov_ps3_abi::lv1::uart`]. CellGov builds the reply only for a
//! request of one of these tables:
//! - [`cellgov_ps3_abi::lv1::uart::GAME_OS_DISPATCHER_REPLIES`], with the
//!   status and the data bytes of its row;
//! - [`cellgov_ps3_abi::lv1::uart::GAME_OS_RANDOM_DISPATCHER_REQUESTS`],
//!   with the status of its row and [`SYNTHETIC_RANDOM_DATA`];
//! - [`cellgov_ps3_abi::lv1::uart::GAME_OS_IDENTITY_DISPATCHER_REQUESTS`],
//!   with the status of its row and the data bytes of a console identity.

use cellgov_ps3_abi::lv1::uart::{
    dispatcher_header_reply_bytes, dispatcher_header_request_bytes, dispatcher_header_service,
    dispatcher_header_tag, dispatcher_reply_header, game_os_dispatcher_reply,
    game_os_identity_dispatcher_request, game_os_random_dispatcher_request,
    DISPATCHER_HEADER_BYTES, DISPATCHER_RANDOM_DATA_BYTES, DISPATCHER_REPLY_DATA_OFFSET,
    DISPATCHER_REPLY_STATUS_OFFSET, GAME_OS_FIRST_DISPATCHER_SERVICE,
};

use super::request::{PendingRequest, REQUEST_PREFIX_BYTES};
use super::write::refuse;
use crate::answer::{HypercallDisposition, HypercallRefusal};
use crate::identity::{ConsoleIdentity, SYNTHETIC_CONSOLE_IDENTITY};
use crate::request::HypercallRequest;

const HEADER: usize = DISPATCHER_HEADER_BYTES as usize;

/// The part of the reply to a request of service 0x19000 that no source
/// gives, for a request id that no source gives a reply of.
pub(super) const SERVICE_REQUEST_PART: &str =
    "a reply of this request, which a service of lv1 gives";

/// The part of the reply to a request of a service that no source gives
/// a reply of.
pub(super) const REQUEST_PART: &str = "a reply of this service, which the dispatcher manager \
                                       relays from the service or gives from its own state";

/// The part of the reply to a request whose sizes are not the sizes of
/// the reply that a source gives.
pub(super) const SIZE_PART: &str = "a reply with these request and reply sizes";

/// The random data bytes that CellGov gives in each reply to a request of
/// [`cellgov_ps3_abi::lv1::uart::GAME_OS_RANDOM_DISPATCHER_REQUESTS`].
///
/// CellGov serves random bytes that lv1 draws as one committed synthetic
/// value, so a run reproduces on every machine.
pub(super) const SYNTHETIC_RANDOM_DATA: [u8; DISPATCHER_RANDOM_DATA_BYTES] =
    *b"CellGov synthetic random";

/// The fields of a reply that a table row gives.
struct ServedReply {
    /// The count of request bytes and the count of reply bytes.
    bytes: u32,
    /// The status word at [`DISPATCHER_REPLY_STATUS_OFFSET`].
    status: u32,
    /// The data bytes at [`DISPATCHER_REPLY_DATA_OFFSET`].
    data: Vec<u8>,
}

/// The reply that CellGov builds for request `request` of service
/// `service` at `firmware`, with the data bytes of `identity` for a
/// request of the identity table. The value is `None` when no table gives
/// the request.
fn served_reply(
    firmware: &str,
    identity: &ConsoleIdentity,
    service: u32,
    request: u64,
) -> Option<ServedReply> {
    if let Some(random) = game_os_random_dispatcher_request(firmware, service, request) {
        return Some(ServedReply {
            bytes: random.bytes,
            status: random.status,
            data: SYNTHETIC_RANDOM_DATA.to_vec(),
        });
    }
    if let Some(row) = game_os_identity_dispatcher_request(firmware, service, request) {
        return Some(ServedReply {
            bytes: row.bytes,
            status: row.status,
            data: identity.dispatcher_data().to_vec(),
        });
    }
    game_os_dispatcher_reply(firmware, service, request).map(|reply| ServedReply {
        bytes: reply.bytes,
        status: reply.status,
        data: reply.data.to_vec(),
    })
}

/// The part of the reply that no source gives, for a request of service
/// `service` that no table serves.
fn unserved_part(service: u32) -> &'static str {
    if service == GAME_OS_FIRST_DISPATCHER_SERVICE {
        SERVICE_REQUEST_PART
    } else {
        REQUEST_PART
    }
}

/// The refusal of a complete request whose bytes the hypervisor did not
/// keep.
pub(super) const UNKEPT_REFUSAL: &str =
    "the reply to a complete request on the dispatcher manager's port whose bytes past the first \
     24 the partition sent in more than one write";

/// The refusal of a write whose bytes complete a request and are more
/// than its bytes.
pub(super) const OVERRUN_REFUSAL: &str =
    "a virtual UART write whose bytes complete a request on the dispatcher manager's port and \
     start a second request";

/// The refusal of a reply to bytes that do not complete a request.
const INCOMPLETE_REFUSAL: &str =
    "the reply to a request on the dispatcher manager's port that the partition did not complete";

/// Why CellGov builds no reply for a complete request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum NoReply {
    /// The partition sent the request's bytes past the first 24 in more
    /// than one write, so the hypervisor did not keep each request byte
    /// that the reply echoes.
    Unkept,
    /// The bytes do not complete the request.
    Incomplete,
    /// The last write holds bytes past the end of the request.
    Overrun,
    /// No source gives a part of the reply to the complete request.
    Unsourced(Unsourced),
}

/// A complete request, and the part of its reply that no source gives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Unsourced {
    /// The service id that the request header gives.
    pub(super) service: u32,
    /// The request id that the request bytes start with, or `None` when
    /// the header counts fewer than 8 request bytes.
    pub(super) request: Option<u64>,
    /// The part of the reply that no source gives.
    pub(super) part: &'static str,
}

impl NoReply {
    /// The refusal of the write `request` that sent the request's last
    /// bytes.
    pub(super) fn refusal(self, request: &HypercallRequest) -> HypercallDisposition {
        let what = match self {
            Self::Unkept => UNKEPT_REFUSAL,
            Self::Incomplete => INCOMPLETE_REFUSAL,
            Self::Overrun => OVERRUN_REFUSAL,
            Self::Unsourced(unsourced) => {
                return HypercallDisposition::Refused(HypercallRefusal::DispatcherRequest {
                    unit: request.unit,
                    pc: request.pc,
                    service: unsourced.service,
                    request: unsourced.request,
                    part: unsourced.part,
                })
            }
        };
        refuse(request, what)
    }
}

/// The bytes that the dispatcher manager writes to its port when the
/// partition's write of `last` completes `pending`, with the data bytes
/// of [`SYNTHETIC_CONSOLE_IDENTITY`]; see [`dispatcher_reply_with`].
///
/// # Errors
///
/// Returns the [`NoReply`] of [`dispatcher_reply_with`].
pub(super) fn dispatcher_reply(
    firmware: &str,
    pending: &PendingRequest,
    last: &[u8],
) -> Result<Vec<u8>, NoReply> {
    dispatcher_reply_with(firmware, &SYNTHETIC_CONSOLE_IDENTITY, pending, last)
}

/// The bytes that the dispatcher manager writes to its port when the
/// partition's write of `last` completes `pending`, with the data bytes
/// of `identity` in a reply to a request of the identity table.
///
/// # Errors
///
/// Returns the [`NoReply`] in these cases:
/// - `pending` holds more bytes than the hypervisor keeps, so CellGov
///   does not hold each request byte that the reply echoes;
/// - `pending` and `last` do not complete the request;
/// - `last` holds bytes past the end of the request;
/// - no table that CellGov builds a reply from gives the header's service
///   and the request id at `firmware`;
/// - the header's counts are not the counts of that reply.
pub(super) fn dispatcher_reply_with(
    firmware: &str,
    identity: &ConsoleIdentity,
    pending: &PendingRequest,
    last: &[u8],
) -> Result<Vec<u8>, NoReply> {
    let Some(kept) = usize::try_from(pending.sent)
        .ok()
        .filter(|&sent| sent <= REQUEST_PREFIX_BYTES)
    else {
        return Err(NoReply::Unkept);
    };
    let bytes = [&pending.prefix[..kept], last].concat();
    let Some(header) = bytes
        .get(..HEADER)
        .and_then(|header| <[u8; HEADER]>::try_from(header).ok())
    else {
        return Err(NoReply::Incomplete);
    };
    let request_bytes = dispatcher_header_request_bytes(&header) as usize;
    match bytes.len().cmp(&(HEADER + request_bytes)) {
        std::cmp::Ordering::Less => return Err(NoReply::Incomplete),
        std::cmp::Ordering::Greater => return Err(NoReply::Overrun),
        std::cmp::Ordering::Equal => {}
    }
    let service = dispatcher_header_service(&header);
    let id = pending.with(last).first_doubleword();
    let unsourced = |part| {
        NoReply::Unsourced(Unsourced {
            service,
            request: id,
            part,
        })
    };
    let reply_bytes = dispatcher_header_reply_bytes(&header);
    let reply = id
        .and_then(|id| served_reply(firmware, identity, service, id))
        .ok_or_else(|| unsourced(unserved_part(service)))?;
    if request_bytes != reply.bytes as usize || reply_bytes != reply.bytes {
        return Err(unsourced(SIZE_PART));
    }
    let mut body = bytes[HEADER..].to_vec();
    let status = DISPATCHER_REPLY_STATUS_OFFSET..DISPATCHER_REPLY_STATUS_OFFSET + 4;
    let data = DISPATCHER_REPLY_DATA_OFFSET..DISPATCHER_REPLY_DATA_OFFSET + reply.data.len();
    body.get_mut(status)
        .ok_or_else(|| unsourced(SIZE_PART))?
        .copy_from_slice(&reply.status.to_be_bytes());
    body.get_mut(data)
        .ok_or_else(|| unsourced(SIZE_PART))?
        .copy_from_slice(&reply.data);
    let tag = dispatcher_header_tag(&header);
    Ok([&dispatcher_reply_header(tag, reply_bytes)[..], &body].concat())
}

#[cfg(test)]
#[path = "tests/reply_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/reply_unsourced_tests.rs"]
mod unsourced_tests;

#[cfg(test)]
#[path = "tests/reply_random_tests.rs"]
mod random_tests;

#[cfg(test)]
#[path = "tests/reply_identity_tests.rs"]
mod identity_tests;

#[cfg(test)]
#[path = "tests/reply_identity_served_tests.rs"]
mod identity_served_tests;
