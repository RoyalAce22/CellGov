//! Local control-flow helpers shared by the LV2 code scanners.
//!
//! Each scanner reads one function's words from the loaded segments and
//! follows branches only inside that word range.

use crate::instruction::PpuInstruction;
use crate::loader::{file_offset_at, LoadSegment};

/// How many words a scanner reads from one function entry.
const MAX_FUNCTION_WORDS: usize = 384;

/// Decodes up to [`MAX_FUNCTION_WORDS`] words at `target`; a word that
/// does not decode is `None`.
pub(crate) fn decode_function(
    elf: &[u8],
    segments: &[LoadSegment],
    target: u64,
) -> Option<Vec<Option<PpuInstruction>>> {
    let words = words_at(elf, segments, target, MAX_FUNCTION_WORDS)?;
    Some(
        words
            .iter()
            .map(|word| crate::decode::decode(*word).ok())
            .collect(),
    )
}

/// Up to `maximum` big-endian words from the executable segment that
/// holds `address`, stopping at the segment's file end.
fn words_at(
    elf: &[u8],
    segments: &[LoadSegment],
    address: u64,
    maximum: usize,
) -> Option<Vec<u32>> {
    let segment = segments.iter().find(|segment| {
        segment.executable
            && address >= segment.vaddr
            && segment
                .vaddr
                .checked_add(segment.filesz)
                .is_some_and(|end| address < end)
    })?;
    let available = segment
        .vaddr
        .checked_add(segment.filesz)?
        .checked_sub(address)? as usize;
    let count = maximum.min(available / 4);
    let offset = file_offset_at(segments, address, count.checked_mul(4)?)?;
    Some(
        elf.get(offset..offset.checked_add(count.checked_mul(4)?)?)?
            .chunks_exact(4)
            .map(|bytes| u32::from_be_bytes(bytes.try_into().expect("four-byte chunk")))
            .collect(),
    )
}

/// The index of the last word before the second stack-frame prologue
/// (`stdu r1, -n(r1)`), or of the last word when there is none.
pub(crate) fn function_extent(decoded: &[Option<PpuInstruction>]) -> usize {
    let mut saw_prologue = false;
    for (index, instruction) in decoded.iter().enumerate() {
        if matches!(
            instruction,
            Some(PpuInstruction::Stdu {
                rs: 1,
                ra: 1,
                imm,
            }) if *imm < 0
        ) {
            if saw_prologue {
                return index.saturating_sub(1);
            }
            saw_prologue = true;
        }
    }
    decoded.len().saturating_sub(1)
}

/// The word index a branch at word `index` reaches, when the target is
/// word-aligned and below `word_count`.
///
/// `base` is the address of word 0; `absolute` is the AA bit.
pub(crate) fn local_branch_index(
    base: u64,
    index: usize,
    displacement: i64,
    absolute: bool,
    word_count: usize,
) -> Option<usize> {
    let instruction = base.checked_add(u64::try_from(index.checked_mul(4)?).ok()?)?;
    let target = if absolute {
        u64::try_from(displacement).ok()?
    } else {
        add_signed(instruction, displacement)?
    };
    let relative = target.checked_sub(base)?;
    if relative % 4 != 0 {
        return None;
    }
    let index = usize::try_from(relative / 4).ok()?;
    (index < word_count).then_some(index)
}

/// `base + displacement`, or `None` when it leaves the u64 range.
pub(crate) fn add_signed(base: u64, displacement: i64) -> Option<u64> {
    if displacement >= 0 {
        base.checked_add(displacement as u64)
    } else {
        base.checked_sub(displacement.unsigned_abs())
    }
}

/// Whether a branch BO field ignores both CTR and the CR bit.
///
/// [PPC-Book1 p:20 s:2.4.1] BO=1z1zz is unconditional.
pub(crate) fn is_unconditional(bo: u8) -> bool {
    bo & 0b10100 == 0b10100
}

/// Whether control can reach the next word after a branch: a conditional
/// branch can fall through, and a linking branch returns there.
pub(crate) fn falls_through(bo: u8, link: bool) -> bool {
    link || !is_unconditional(bo)
}
