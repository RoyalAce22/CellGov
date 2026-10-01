//! The CGOV frame a microtest emits, and the parser that slices its
//! regions out.
//!
//! The test writes `CGOV` (4 bytes) + a big-endian u32 payload length +
//! the raw payload bytes: to the TTY under an emulator, to a file on a
//! console. The parser locates the magic in the bytes it is given and
//! slices each region from the payload at its stated offset.
//!
//! One input carries exactly one frame. A second frame past the first
//! one's payload is refused rather than resolved by position.

use crate::observation::NamedMemoryRegion;

/// Magic tag that precedes the big-endian u32 length and payload bytes.
pub const FRAME_MAGIC: &[u8; 4] = b"CGOV";

/// Frame header: 4-byte magic + 4-byte length.
const FRAME_HEADER_SIZE: usize = 8;

/// A region to slice out of the frame payload.
#[derive(Debug, Clone)]
pub struct FrameRegion {
    /// Region name.
    pub name: String,
    /// Byte offset within the payload.
    ///
    /// Stated rather than accumulated: a guest emits one struct and
    /// names positions inside it, so the regions can leave alignment
    /// padding between them. Summing sizes would slide every region
    /// after the first gap.
    pub offset: u64,
    /// Number of bytes for this region within the payload.
    pub size: u64,
    /// Guest address to report in the observation.
    pub guest_addr: u64,
}

/// Why the bytes did not yield the declared regions.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The bytes do not contain the magic tag.
    #[error("CGOV frame: magic tag not found")]
    MagicNotFound,
    /// A second `CGOV` frame follows the one that was parsed.
    ///
    /// Two frames outside one another's payload mean either the guest
    /// emitted twice or the input outlived a run. Picking the first is
    /// a guess, and the wrong guess returns plausible neighbouring
    /// bytes.
    #[error(
        "CGOV frame at byte {first} is followed by a second CGOV frame at byte {second}; \
         clear the output before the run so one capture is unambiguous"
    )]
    Ambiguous {
        /// Byte offset of the frame that was parsed.
        first: usize,
        /// Byte offset of the next frame found past its payload.
        second: usize,
    },
    /// The header or payload is shorter than it or the declared regions
    /// require.
    #[error("CGOV frame payload too small: expected >= {expected} bytes, got {actual}")]
    PayloadTooSmall {
        /// Minimum size required.
        expected: u64,
        /// Actual size.
        actual: u64,
    },
    /// A region's offset plus size overflows `u64`.
    #[error(
        "CGOV frame region {region_name:?} offset/size overflow: offset={offset}, size={size}"
    )]
    OffsetOverflow {
        /// Region name.
        region_name: String,
        /// Payload offset declared for the region.
        offset: u64,
        /// Region byte length.
        size: u64,
    },
}

/// Byte offset of the first `CGOV` at or after `from`.
fn find_magic(data: &[u8], from: usize) -> Option<usize> {
    data.get(from..)?
        .windows(FRAME_MAGIC.len())
        .position(|w| w == FRAME_MAGIC.as_slice())
        .map(|p| p + from)
}

/// Scan `data`, a whole TTY log or a bare frame, for the `CGOV` frame
/// and slice each declared region out of its payload at that region's
/// offset.
///
/// # Errors
///
/// Returns `Err` when the bytes carry no frame, carry a second frame
/// past the first one's payload, are truncated inside the header or
/// payload, or declare a region the payload cannot satisfy.
pub fn parse_frame(
    data: &[u8],
    regions: &[FrameRegion],
) -> Result<Vec<NamedMemoryRegion>, FrameError> {
    let magic_pos = find_magic(data, 0).ok_or(FrameError::MagicNotFound)?;

    // `magic_pos < data.len()` by find-position contract, so the
    // header_end add is bounded by `data.len() + FRAME_HEADER_SIZE`,
    // well below `usize::MAX` for any real input.
    let header_end = magic_pos + FRAME_HEADER_SIZE;
    let Some(len_bytes) = data.get(magic_pos + 4..header_end) else {
        return Err(FrameError::PayloadTooSmall {
            expected: FRAME_HEADER_SIZE as u64,
            actual: (data.len() - magic_pos) as u64,
        });
    };
    let payload_len = u64::from(u32::from_be_bytes([
        len_bytes[0],
        len_bytes[1],
        len_bytes[2],
        len_bytes[3],
    ]));

    let payload_start = header_end;
    let available = (data.len() - payload_start) as u64;
    if payload_len > available {
        return Err(FrameError::PayloadTooSmall {
            expected: payload_len,
            actual: available,
        });
    }
    // payload_len <= available, which came from a usize.
    let payload_end = payload_start + payload_len as usize;
    // Magic bytes inside the payload are region data. One that starts
    // past the payload is a second frame, and nothing here can tell
    // which of the two the caller meant.
    if let Some(second) = find_magic(data, payload_end) {
        return Err(FrameError::Ambiguous {
            first: magic_pos,
            second,
        });
    }
    let payload = &data[payload_start..payload_end];

    let mut result = Vec::with_capacity(regions.len());
    for region in regions {
        let region_end =
            region
                .offset
                .checked_add(region.size)
                .ok_or_else(|| FrameError::OffsetOverflow {
                    region_name: region.name.clone(),
                    offset: region.offset,
                    size: region.size,
                })?;
        if region_end > payload_len {
            return Err(FrameError::PayloadTooSmall {
                expected: region_end,
                actual: payload_len,
            });
        }
        // region_end <= payload_len <= u32::MAX, so both bounds stay
        // within usize on all supported hosts.
        let lo = region.offset as usize;
        let hi = region_end as usize;
        result.push(NamedMemoryRegion {
            name: region.name.clone(),
            addr: region.guest_addr,
            data: payload[lo..hi].to_vec(),
        });
    }
    Ok(result)
}

#[cfg(test)]
#[path = "tests/frame_tests.rs"]
mod tests;
