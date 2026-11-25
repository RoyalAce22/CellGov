//! The bytes in the receive buffer of a service port that the partition
//! did not read yet.
//!
//! The dispatcher manager writes a service's reply to its port; see
//! [`cellgov_ps3_abi::lv1::uart`]. `read_virtual_uart` reads the bytes in
//! the order that they came, and lv1's status function counts them.

use cellgov_mem::lanes::{LaneValue, ObjectLanes};

/// The bytes in the receive buffer of one port.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedBytes {
    /// The port number.
    pub port: u64,
    /// The bytes, first byte to read first.
    pub bytes: Vec<u8>,
}

impl ReceivedBytes {
    /// The receive buffer of port `port` when it holds no byte.
    pub const fn empty(port: u64) -> Self {
        Self {
            port,
            bytes: Vec::new(),
        }
    }

    /// The count of bytes in the buffer.
    pub fn count(&self) -> u64 {
        self.bytes.len() as u64
    }
}

/// Fields 1 to 3: the port number, the count of bytes, and the bytes as
/// big-endian doublewords, one slot each in order. The last doubleword
/// holds 0 in each byte past the count.
impl LaneValue for ReceivedBytes {
    fn lanes(&self, lanes: &mut ObjectLanes) {
        lanes.lane(1, 0, self.port);
        lanes.lane(2, 0, self.count());
        for (slot, chunk) in (0u64..).zip(self.bytes.chunks(8)) {
            let mut doubleword = [0; 8];
            doubleword[..chunk.len()].copy_from_slice(chunk);
            lanes.lane(3, slot, u64::from_be_bytes(doubleword));
        }
    }
}

#[cfg(test)]
#[path = "tests/receive_hash_tests.rs"]
mod hash_tests;
