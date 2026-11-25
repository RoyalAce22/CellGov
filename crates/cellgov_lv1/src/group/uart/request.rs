//! The bytes that the partition sent of a request on a service port,
//! while they do not hold the whole request.
//!
//! The service reads a header and the bytes that the header counts, and
//! only then replies; see [`cellgov_ps3_abi::lv1::uart`]. The hypervisor
//! keeps the bytes of the request that is not complete, so that it knows
//! which write completes it. The methods that decode the header read the
//! dispatcher manager's header. The system manager's header has the same
//! size and different fields; see `super::system_manager`.

use cellgov_mem::lanes::{LaneValue, ObjectLanes};
use cellgov_ps3_abi::lv1::uart::{
    dispatcher_header_request_bytes, dispatcher_header_service, DISPATCHER_HEADER_BYTES,
};

/// The bytes of a request that the hypervisor keeps: the header and the
/// first doubleword of the request bytes.
pub const REQUEST_PREFIX_BYTES: usize = DISPATCHER_HEADER_BYTES as usize + 8;

const HEADER: usize = DISPATCHER_HEADER_BYTES as usize;

/// The bytes that the partition sent of one request on a port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PendingRequest {
    /// The port number.
    pub port: u64,
    /// The count of bytes that the partition sent of the request.
    pub sent: u64,
    /// The first bytes of the request. A byte that the partition did not
    /// send is 0.
    pub prefix: [u8; REQUEST_PREFIX_BYTES],
}

impl PendingRequest {
    /// The request of port `port` before the partition sends a byte.
    pub const fn empty(port: u64) -> Self {
        Self {
            port,
            sent: 0,
            prefix: [0; REQUEST_PREFIX_BYTES],
        }
    }

    /// The request after the partition sends `bytes` more.
    pub(super) fn with(&self, bytes: &[u8]) -> Self {
        let mut next = *self;
        let start = usize::try_from(self.sent).unwrap_or(usize::MAX);
        for (slot, byte) in next.prefix.iter_mut().skip(start).zip(bytes) {
            *slot = *byte;
        }
        next.sent = self.sent.saturating_add(bytes.len() as u64);
        next
    }

    /// The header, or `None` before the partition sends each of its bytes.
    fn header(&self) -> Option<[u8; HEADER]> {
        if self.sent < DISPATCHER_HEADER_BYTES {
            return None;
        }
        let mut header = [0; HEADER];
        header.copy_from_slice(&self.prefix[..HEADER]);
        Some(header)
    }

    /// The service id that the header gives, or `None` before the header
    /// is complete.
    pub fn service(&self) -> Option<u32> {
        self.header()
            .map(|header| dispatcher_header_service(&header))
    }

    /// The first doubleword of the request bytes.
    ///
    /// The value is `None` in these cases:
    /// - the partition did not send the doubleword;
    /// - the header counts fewer than 8 request bytes.
    pub fn first_doubleword(&self) -> Option<u64> {
        let header = self.header()?;
        if dispatcher_header_request_bytes(&header) < 8 || self.sent < REQUEST_PREFIX_BYTES as u64 {
            return None;
        }
        let mut doubleword = [0; 8];
        doubleword.copy_from_slice(&self.prefix[HEADER..]);
        Some(u64::from_be_bytes(doubleword))
    }

    /// Whether the partition sent the header and each request byte that it
    /// counts.
    pub fn is_complete(&self) -> bool {
        self.header().is_some_and(|header| {
            self.sent
                >= DISPATCHER_HEADER_BYTES + u64::from(dispatcher_header_request_bytes(&header))
        })
    }
}

/// Fields 1 to 5: the port number, the count of bytes sent, and the three
/// doublewords of the prefix in order.
impl LaneValue for PendingRequest {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.port);
        lanes.lane(2, 0, self.sent);
        for (field, chunk) in (3u8..).zip(self.prefix.chunks_exact(8)) {
            let mut doubleword = [0; 8];
            doubleword.copy_from_slice(chunk);
            lanes.lane(field, 0, u64::from_be_bytes(doubleword));
        }
    }
}

#[cfg(test)]
#[path = "tests/request_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tests/request_hash_tests.rs"]
mod hash_tests;
