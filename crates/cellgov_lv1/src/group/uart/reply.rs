//! The bytes that the dispatcher manager writes to its port for a
//! complete request: a reply header, then the service's reply bytes.
//!
//! The service writes its reply fields over the request bytes, so each
//! other byte of the reply is the request byte at the same offset; see
//! [`cellgov_ps3_abi::lv1::uart`]. CellGov builds the reply only for a
//! request that [`cellgov_ps3_abi::lv1::uart::GAME_OS_DISPATCHER_REPLIES`]
//! gives.

use cellgov_ps3_abi::lv1::uart::{
    dispatcher_header_reply_bytes, dispatcher_header_request_bytes, dispatcher_header_service,
    dispatcher_header_tag, dispatcher_reply_header, game_os_dispatcher_reply,
    DISPATCHER_HEADER_BYTES, DISPATCHER_REPLY_DATA_BYTES, DISPATCHER_REPLY_DATA_OFFSET,
    DISPATCHER_REPLY_STATUS_OFFSET, GAME_OS_FIRST_DISPATCHER_SERVICE,
};

use super::request::{PendingRequest, REQUEST_PREFIX_BYTES};

const HEADER: usize = DISPATCHER_HEADER_BYTES as usize;

/// The refusal of a complete request of service 0x19000 that no source
/// gives the reply to.
pub(super) const SERVICE_REQUEST_REFUSAL: &str =
    "the reply to a request of service 0x19000 other than request 0x19002 on the dispatcher \
     manager's port, which a service of lv1 gives";

/// The refusal of a complete request of a service that no source gives a
/// reply of.
pub(super) const REQUEST_REFUSAL: &str = "the reply to a complete request on the dispatcher \
                                          manager's port, which the dispatcher manager relays \
                                          from a service or gives from its own state";

/// The refusal of a complete request whose sizes are not the sizes of
/// the reply that a source gives.
pub(super) const SIZE_REFUSAL: &str = "the reply to request 0x19002 of service 0x19000 on the \
                                       dispatcher manager's port, for request or reply sizes \
                                       other than 0x38";

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
    /// No source gives the reply to the request of service 0x19000.
    ServiceRequest,
    /// No source gives a reply of the request's service.
    Request,
    /// The header's counts are not the counts of the reply.
    Size,
}

impl NoReply {
    /// The inputs that the refusal names.
    pub(super) const fn what(self) -> &'static str {
        match self {
            Self::Unkept => UNKEPT_REFUSAL,
            Self::Incomplete => INCOMPLETE_REFUSAL,
            Self::Overrun => OVERRUN_REFUSAL,
            Self::ServiceRequest => SERVICE_REQUEST_REFUSAL,
            Self::Request => REQUEST_REFUSAL,
            Self::Size => SIZE_REFUSAL,
        }
    }
}

/// The bytes that the dispatcher manager writes to its port when the
/// partition's write of `last` completes `pending`.
///
/// # Errors
///
/// Returns the [`NoReply`] in these cases:
/// - `pending` holds more bytes than the hypervisor keeps, so CellGov
///   does not hold each request byte that the reply echoes;
/// - `pending` and `last` do not complete the request;
/// - `last` holds bytes past the end of the request;
/// - no source gives the reply of the header's service and the request
///   id at `firmware`;
/// - the header's counts are not the counts of that reply.
pub(super) fn dispatcher_reply(
    firmware: &str,
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
    let reply = pending
        .with(last)
        .first_doubleword()
        .and_then(|request| game_os_dispatcher_reply(firmware, service, request))
        .ok_or(if service == GAME_OS_FIRST_DISPATCHER_SERVICE {
            NoReply::ServiceRequest
        } else {
            NoReply::Request
        })?;
    let reply_bytes = dispatcher_header_reply_bytes(&header);
    if request_bytes != reply.bytes as usize || reply_bytes != reply.bytes {
        return Err(NoReply::Size);
    }
    let mut body = bytes[HEADER..].to_vec();
    let status = DISPATCHER_REPLY_STATUS_OFFSET..DISPATCHER_REPLY_STATUS_OFFSET + 4;
    let data =
        DISPATCHER_REPLY_DATA_OFFSET..DISPATCHER_REPLY_DATA_OFFSET + DISPATCHER_REPLY_DATA_BYTES;
    body.get_mut(status)
        .ok_or(NoReply::Size)?
        .copy_from_slice(&reply.status.to_be_bytes());
    body.get_mut(data)
        .ok_or(NoReply::Size)?
        .copy_from_slice(&reply.data);
    let tag = dispatcher_header_tag(&header);
    Ok([&dispatcher_reply_header(tag, reply_bytes)[..], &body].concat())
}

#[cfg(test)]
#[path = "tests/reply_tests.rs"]
mod tests;
