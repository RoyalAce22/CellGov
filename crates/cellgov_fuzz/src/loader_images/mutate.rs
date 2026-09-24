//! The byte-level corruptions and the stream-described image.

use super::elf::{put_u32, put_u64, ExecImage};
use super::prx::PrxImage;
use super::stream::FieldStream;

/// Apply up to three byte-level corruptions the stream selects.
pub fn corrupt(bytes: &mut Vec<u8>, s: &mut FieldStream<'_>) {
    let rounds = s.below(4);
    for _ in 0..rounds {
        let len = bytes.len();
        if len == 0 {
            return;
        }
        let pos = s.below(len as u32) as usize;
        match s.below(6) {
            0 => bytes.truncate(pos),
            1 => bytes[pos] ^= 1 << s.below(8),
            2 => bytes[pos] = [0, 0xFF, 0x7F, 0x80][s.below(4) as usize],
            3 => {
                let at = (pos & !3).min(len.saturating_sub(4));
                if at + 4 <= len {
                    put_u32(bytes, at, s.u32());
                }
            }
            4 => {
                let at = (pos & !7).min(len.saturating_sub(8));
                if at + 8 <= len {
                    put_u64(bytes, at, s.interesting_u64());
                }
            }
            _ => {
                let count = s.below(16) as usize;
                let insert = s.bytes(count);
                bytes.splice(pos..pos, insert);
            }
        }
    }
}

/// The image a fuzz byte stream describes: an executable or a module,
/// rendered and then corrupted as the stream says.
pub fn structured_image(data: &[u8]) -> Vec<u8> {
    let mut s = FieldStream::new(data);
    let mut bytes = if s.u8() & 1 == 0 {
        ExecImage::from_stream(&mut s).render()
    } else {
        PrxImage::from_stream(&mut s).render()
    };
    corrupt(&mut bytes, &mut s);
    bytes
}

#[cfg(test)]
#[path = "tests/mutate_tests.rs"]
mod tests;
