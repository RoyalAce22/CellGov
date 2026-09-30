//! A running unit's local store and program counter, written as one file.
//!
//! The file is the eight-byte magic `CGSPULS1`, the PC as a big-endian
//! word, then the whole local store. It holds code that a unit received by
//! DMA or unpacked at run time, which no stored image carries.

use crate::state::{SpuState, SPU_LS_SIZE};

/// The first bytes of a capture file.
const LOCAL_STORE_CAPTURE_MAGIC: [u8; 8] = *b"CGSPULS1";

/// Bytes before the local store: the magic and the PC.
const HEADER: usize = LOCAL_STORE_CAPTURE_MAGIC.len() + 4;

/// One unit's local store and the address of its next instruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalStoreCapture {
    /// The address of the unit's next instruction.
    pub pc: u32,
    /// The whole local store.
    pub local_store: Vec<u8>,
}

/// Why bytes that start with the capture magic are not a capture.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocalStoreCaptureError {
    /// The bytes do not start with the magic `CGSPULS1`.
    #[error("the file does not start with the local-store capture magic")]
    NotACapture,
    /// The file is not the header and one whole local store.
    #[error("a local-store capture is {expected} bytes, not {found}")]
    Length {
        /// The length of a capture.
        expected: usize,
        /// The file's length.
        found: usize,
    },
}

impl LocalStoreCapture {
    /// The capture of `state`.
    pub fn of(state: &SpuState) -> Self {
        Self {
            pc: state.pc,
            local_store: state.ls.clone(),
        }
    }

    /// The capture as file bytes.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(HEADER + self.local_store.len());
        bytes.extend_from_slice(&LOCAL_STORE_CAPTURE_MAGIC);
        bytes.extend_from_slice(&self.pc.to_be_bytes());
        bytes.extend_from_slice(&self.local_store);
        bytes
    }

    /// Whether `bytes` start with the capture magic.
    pub fn is_capture(bytes: &[u8]) -> bool {
        bytes.starts_with(&LOCAL_STORE_CAPTURE_MAGIC)
    }

    /// Reads a capture file.
    ///
    /// # Errors
    ///
    /// [`LocalStoreCaptureError`] when the magic is missing or the file
    /// is not one whole local store long.
    pub fn parse(bytes: &[u8]) -> Result<Self, LocalStoreCaptureError> {
        if !Self::is_capture(bytes) {
            return Err(LocalStoreCaptureError::NotACapture);
        }
        if bytes.len() != HEADER + SPU_LS_SIZE {
            return Err(LocalStoreCaptureError::Length {
                expected: HEADER + SPU_LS_SIZE,
                found: bytes.len(),
            });
        }
        let pc = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        Ok(Self {
            pc,
            local_store: bytes[HEADER..].to_vec(),
        })
    }
}

#[cfg(test)]
#[path = "tests/capture_tests.rs"]
mod tests;
