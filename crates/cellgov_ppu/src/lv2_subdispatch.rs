//! Discovers packet dispatch below LV2 syscall-table targets.

use std::collections::{BTreeMap, BTreeSet};

use cellgov_ps3_abi::lv2::syscall;

use crate::instruction::PpuInstruction;
use crate::loader::{pt_load_segments, LoadError, LoadSegment};
use crate::lv2_stub::{self, Lv2StubClassification, Lv2StubClassificationError};

const MAX_FUNCTION_WORDS: usize = 384;
const MAX_SUBENTRIES: usize = 256;

/// Records how packet discovery interprets one code target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2SubentryClass {
    /// The packet target follows the jump table.
    Implemented,
    /// The packet target is the shared refusal before the jump table.
    Stub,
}

/// Records one packet selected below an LV2 ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv2Subentry {
    /// The kernel compares this packet value.
    pub packet: u64,
    /// Records how packet discovery interprets the target.
    pub class: Lv2SubentryClass,
    /// The packet selects this code address.
    pub target: u64,
}

/// Records the packet-dispatch shape below one LV2 ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Lv2Subdispatch {
    /// The scanner decoded a bounded relative-offset jump table.
    Table {
        /// Names the zero-based syscall argument slot that selects the packet (`0` is r3).
        selector_slot: usize,
        /// Lists packet rows in ascending packet order.
        entries: Vec<Lv2Subentry>,
    },
    /// The scanner found repeated argument comparisons without a jump table.
    ChainIncomplete {
        /// Names the zero-based syscall argument slot that selects the packet (`0` is r3).
        selector_slot: usize,
    },
}

/// Combines packet-dispatch results with the top-level classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lv2SubdispatchClassification {
    /// Preserves the classification produced before packet discovery.
    pub top_level: Lv2StubClassification,
    /// Omits ordinals without recognized packet dispatch.
    pub ordinals: BTreeMap<usize, Lv2Subdispatch>,
}

/// Reports why packet-dispatch discovery refused an ELF.
#[derive(Debug, thiserror::Error)]
pub enum Lv2SubdispatchError {
    /// The top-level classifier refused the ELF.
    #[error("LV2 sub-dispatch discovery: {0}")]
    Classification(#[from] Lv2StubClassificationError),
    /// The loader failed to read the ELF load segments.
    #[error("LV2 sub-dispatch discovery: {0}")]
    Elf(#[from] LoadError),
}

/// Classifies the top-level table and all recognizable packet dispatchers.
///
/// # Errors
///
/// Returns [`Lv2SubdispatchError`] if:
///
/// - top-level classification fails; or
/// - the scanner cannot read the ELF load segments.
pub fn classify(elf: &[u8]) -> Result<Lv2SubdispatchClassification, Lv2SubdispatchError> {
    let top_level = lv2_stub::classify(elf)?;
    let segments = pt_load_segments(elf)?;
    let mut by_target = BTreeMap::new();
    let mut ordinals = BTreeMap::new();
    for ordinal in &top_level.ordinals {
        let Some(target) = ordinal.code else {
            continue;
        };
        let found = by_target
            .entry(target)
            .or_insert_with(|| scan_target(elf, &segments, top_level.discovery.toc, target));
        if matches!(
            found,
            Some(Lv2Subdispatch::ChainIncomplete { selector_slot })
                if *selector_slot != 0
                    && ordinal.ordinal as u64 != syscall::SYS_RSX_CONTEXT_ATTRIBUTE
        ) {
            continue;
        }
        if let Some(dispatch) = found {
            ordinals.insert(ordinal.ordinal, dispatch.clone());
        }
    }
    Ok(Lv2SubdispatchClassification {
        top_level,
        ordinals,
    })
}

#[derive(Debug, Clone, Copy)]
struct Linear {
    slot: usize,
    delta: i64,
}

fn scan_target(
    elf: &[u8],
    segments: &[LoadSegment],
    toc: u64,
    target: u64,
) -> Option<Lv2Subdispatch> {
    let words = words_at(elf, segments, target, MAX_FUNCTION_WORDS)?;
    let decoded: Vec<Option<PpuInstruction>> = words
        .iter()
        .map(|word| crate::decode::decode(*word).ok())
        .collect();
    let reachable = reachable_indices(&decoded, target);
    let extent = function_extent(&decoded);
    let mut linear = [None; 32];
    for slot in 0..8usize {
        linear[3 + slot] = Some(Linear { slot, delta: 0 });
    }
    let mut compared: BTreeMap<usize, BTreeSet<i64>> = BTreeMap::new();
    for (index, instruction) in decoded.iter().enumerate() {
        if index > extent {
            continue;
        }
        let Some(instruction) = instruction else {
            continue;
        };
        if let Some((bf, register, bound)) = unsigned_compare(instruction) {
            if let Some(selector) = linear[register as usize] {
                if let Some(table) = jump_table_after(
                    elf, segments, toc, &decoded, &reachable, index, register, selector, bound,
                ) {
                    return Some(table);
                }
                if branches_on_equal_after(&decoded, index, bf) {
                    compared.entry(selector.slot).or_default().insert(bound);
                }
            }
        }
        update_linear(&mut linear, instruction);
    }
    compared
        .into_iter()
        .filter(|(_, values)| values.len() >= 3)
        .max_by_key(|(slot, values)| (values.len(), std::cmp::Reverse(*slot)))
        .map(|(selector_slot, _)| Lv2Subdispatch::ChainIncomplete { selector_slot })
}

fn function_extent(decoded: &[Option<PpuInstruction>]) -> usize {
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

fn unsigned_compare(instruction: &PpuInstruction) -> Option<(u8, u8, i64)> {
    match *instruction {
        PpuInstruction::Cmplwi { bf, ra, imm } | PpuInstruction::Cmpldi { bf, ra, imm } => {
            Some((bf, ra, i64::from(imm)))
        }
        PpuInstruction::Cmpwi { bf, ra, imm } | PpuInstruction::Cmpdi { bf, ra, imm } => {
            Some((bf, ra, i64::from(imm)))
        }
        _ => None,
    }
}

fn branches_on_equal(instruction: Option<PpuInstruction>, bf: u8) -> bool {
    matches!(
        instruction,
        Some(PpuInstruction::Bc {
            bo,
            bi,
            aa: false,
            link: false,
            ..
        }) if bo & 0b11100 == 0b01100
            && bo & 0b11 != 0b01
            && bi == bf * 4 + 2
    )
}

fn branches_on_equal_after(decoded: &[Option<PpuInstruction>], index: usize, bf: u8) -> bool {
    decoded
        .iter()
        .skip(index + 1)
        .take(4)
        .any(|instruction| branches_on_equal(*instruction, bf))
}

#[allow(clippy::too_many_arguments)]
fn jump_table_after(
    elf: &[u8],
    segments: &[LoadSegment],
    toc: u64,
    decoded: &[Option<PpuInstruction>],
    reachable: &BTreeSet<usize>,
    compare_index: usize,
    index_register: u8,
    selector: Linear,
    bound: i64,
) -> Option<Lv2Subdispatch> {
    let end = (compare_index + 64).min(decoded.len());
    for start in compare_index + 1..end.saturating_sub(6) {
        if !(start..start + 7).all(|index| reachable.contains(&index)) {
            continue;
        }
        let window = &decoded[start..start + 7];
        let (
            Some(PpuInstruction::Ld {
                rt: table_register,
                ra: 2,
                imm,
            }),
            shift,
            Some(PpuInstruction::Lwzx { rt: offset, ra, rb }),
            Some(PpuInstruction::Extsw {
                ra: extended,
                rs: offset_source,
                ..
            }),
            Some(PpuInstruction::Add {
                rt: destination,
                ra: add_left,
                rb: add_right,
                ..
            }),
            Some(PpuInstruction::Mtctr { rs: ctr_source }),
            Some(PpuInstruction::Bcctr {
                bo, link: false, ..
            }),
        ) = (
            window[0], window[1], window[2], window[3], window[4], window[5], window[6],
        )
        else {
            continue;
        };
        let Some((shifted, rs)) = shift_by_two(shift) else {
            continue;
        };
        if rs != index_register
            || ra != shifted
            || rb != table_register
            || extended != offset
            || offset_source != offset
            || destination != offset
            || add_left != offset
            || add_right != table_register
            || ctr_source != offset
            || bo & 0b10100 != 0b10100
        {
            continue;
        }
        let packet_first = selector.delta.checked_neg()?;
        let count = usize::try_from(bound.checked_add(1)?).ok()?;
        if packet_first < 0 || count == 0 || count > MAX_SUBENTRIES {
            continue;
        }
        let pointer_address = add_signed(toc, i64::from(imm))?;
        let table = read_u64_at(elf, segments, pointer_address)?;
        let mut targets = Vec::with_capacity(count);
        let mut references = BTreeMap::new();
        for item in 0..count {
            let address = table.checked_add(u64::try_from(item.checked_mul(4)?).ok()?)?;
            let relative = i64::from(read_i32_at(elf, segments, address)?);
            let target = add_signed(table, relative)?;
            *references.entry(target).or_insert(0usize) += 1;
            targets.push((relative, target));
        }
        let entries = targets
            .into_iter()
            .enumerate()
            .map(|(item, (relative, target))| {
                let packet = u64::try_from(packet_first).ok()?.checked_add(item as u64)?;
                Some(Lv2Subentry {
                    packet,
                    class: if relative < 0 && references[&target] > 1 {
                        Lv2SubentryClass::Stub
                    } else {
                        Lv2SubentryClass::Implemented
                    },
                    target,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        return Some(Lv2Subdispatch::Table {
            selector_slot: selector.slot,
            entries,
        });
    }
    None
}

fn reachable_indices(decoded: &[Option<PpuInstruction>], base: u64) -> BTreeSet<usize> {
    let mut reachable = BTreeSet::new();
    let mut pending = vec![0usize];
    while let Some(start) = pending.pop() {
        let mut index = start;
        while index < decoded.len() && reachable.insert(index) {
            match decoded[index] {
                Some(PpuInstruction::B { offset, aa, link }) => {
                    if link {
                        index += 1;
                        continue;
                    }
                    if let Some(target) =
                        local_branch_index(base, index, i64::from(offset), aa, decoded.len())
                    {
                        pending.push(target);
                    }
                    break;
                }
                Some(PpuInstruction::Bc {
                    bo,
                    offset,
                    aa,
                    link,
                    ..
                }) => {
                    if link || bo & 0b10100 != 0b10100 {
                        pending.push(index + 1);
                    }
                    if !link {
                        if let Some(target) =
                            local_branch_index(base, index, i64::from(offset), aa, decoded.len())
                        {
                            pending.push(target);
                        }
                    }
                    break;
                }
                Some(
                    PpuInstruction::Bclr { bo, link, .. } | PpuInstruction::Bcctr { bo, link, .. },
                ) => {
                    // [PPC-Book1 p:25 s:2.4] A conditional indirect branch falls through when its BO/BI condition is false; LK also records its return site.
                    if link || bo & 0b10100 != 0b10100 {
                        index += 1;
                    } else {
                        break;
                    }
                }
                _ => index += 1,
            }
        }
    }
    reachable
}

fn local_branch_index(
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

fn shift_by_two(instruction: Option<PpuInstruction>) -> Option<(u8, u8)> {
    match instruction? {
        PpuInstruction::Rldicr {
            ra,
            rs,
            sh: 2,
            me: 61,
            ..
        }
        | PpuInstruction::Rldic {
            ra,
            rs,
            sh: 2,
            mb: 30,
            ..
        } => Some((ra, rs)),
        _ => None,
    }
}

fn update_linear(linear: &mut [Option<Linear>; 32], instruction: &PpuInstruction) {
    match *instruction {
        PpuInstruction::Or {
            ra,
            rs,
            rb,
            rc: false,
        } if rs == rb => linear[ra as usize] = linear[rs as usize],
        PpuInstruction::Addi { rt, ra, imm } if ra != 0 => {
            linear[rt as usize] = linear[ra as usize].and_then(|value| {
                value.delta.checked_add(i64::from(imm)).map(|delta| Linear {
                    slot: value.slot,
                    delta,
                })
            });
        }
        PpuInstruction::Addis { rt, ra, imm } if ra != 0 => {
            linear[rt as usize] = linear[ra as usize].and_then(|value| {
                value
                    .delta
                    .checked_add(i64::from(imm) << 16)
                    .map(|delta| Linear {
                        slot: value.slot,
                        delta,
                    })
            });
        }
        PpuInstruction::B { link: true, .. }
        | PpuInstruction::Bc { link: true, .. }
        | PpuInstruction::Bclr { link: true, .. }
        | PpuInstruction::Bcctr { link: true, .. } => {
            linear[..13].fill(None);
        }
        _ => {}
    }
}

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

fn read_u64_at(elf: &[u8], segments: &[LoadSegment], address: u64) -> Option<u64> {
    let offset = file_offset_at(segments, address, 8)?;
    Some(u64::from_be_bytes(
        elf.get(offset..offset + 8)?.try_into().ok()?,
    ))
}

fn read_i32_at(elf: &[u8], segments: &[LoadSegment], address: u64) -> Option<i32> {
    let offset = file_offset_at(segments, address, 4)?;
    Some(i32::from_be_bytes(
        elf.get(offset..offset + 4)?.try_into().ok()?,
    ))
}

fn file_offset_at(segments: &[LoadSegment], address: u64, size: usize) -> Option<usize> {
    let size = u64::try_from(size).ok()?;
    for segment in segments {
        let Some(relative) = address.checked_sub(segment.vaddr) else {
            continue;
        };
        let Some(end) = relative.checked_add(size) else {
            continue;
        };
        if end <= segment.filesz {
            let offset = segment.file_offset.checked_add(relative)?;
            return usize::try_from(offset).ok();
        }
    }
    None
}

fn add_signed(base: u64, displacement: i64) -> Option<u64> {
    if displacement >= 0 {
        base.checked_add(displacement as u64)
    } else {
        base.checked_sub(displacement.unsigned_abs())
    }
}

#[cfg(test)]
#[path = "tests/lv2_subdispatch_tests.rs"]
mod tests;
