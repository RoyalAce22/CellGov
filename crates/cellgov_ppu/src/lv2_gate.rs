//! Discovers capability checks in LV2 syscall implementations.

use std::collections::BTreeMap;

use cellgov_ps3_abi::lv2::errno;

use crate::instruction::PpuInstruction;
use crate::loader::{file_offset_at, pt_load_segments, LoadError, LoadSegment};
use crate::lv2_stub::{Lv2OrdinalClass, Lv2StubClassification};

const MAX_FUNCTION_WORDS: usize = 384;
const MAX_ENTRY_THUNKS: usize = 4;
const PERMISSION_RECORD_BYTES: usize = 32;

/// Describes the capability value read by a recognized check.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2GateRead {
    /// The check reads these bits from `ctrl_flags1`.
    ControlFlags1(u32),
}

/// Describes capability gating for one LV2 ordinal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2Gate {
    /// A recognized permission record requires a capability value.
    Gated {
        /// The value read by the check.
        reads: Lv2GateRead,
        /// The Cell error returned when the check fails.
        fail_errno: u32,
    },
    /// A recognized permission record contains no capability requirement.
    Ungated,
    /// The implementation did not match the bounded recognizer.
    NotAnalysed,
}

/// Classifies capability gating for every LV2 ordinal.
///
/// # Errors
///
/// Returns [`LoadError`] if the ELF load segments cannot be read.
pub fn classify(
    elf: &[u8],
    top_level: &Lv2StubClassification,
) -> Result<BTreeMap<usize, Lv2Gate>, LoadError> {
    let segments = pt_load_segments(elf)?;
    let mut by_target = BTreeMap::new();
    let mut rows = BTreeMap::new();
    for ordinal in &top_level.ordinals {
        let gate = match (ordinal.class, ordinal.code) {
            (Lv2OrdinalClass::Implemented, Some(target)) => *by_target
                .entry(target)
                .or_insert_with(|| scan_target(elf, &segments, top_level.discovery.toc, target)),
            _ => Lv2Gate::NotAnalysed,
        };
        rows.insert(ordinal.ordinal, gate);
    }
    Ok(rows)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Candidate {
    Gated { mask: u32, fail_errno: u32 },
    Ungated,
}

fn scan_target(elf: &[u8], segments: &[LoadSegment], toc: u64, target: u64) -> Lv2Gate {
    let mut target = target;
    for _ in 0..MAX_ENTRY_THUNKS {
        let Some(words) = words_at(elf, segments, target, MAX_FUNCTION_WORDS) else {
            return Lv2Gate::NotAnalysed;
        };
        let decoded: Vec<Option<PpuInstruction>> = words
            .iter()
            .map(|word| crate::decode::decode(*word).ok())
            .collect();
        if let Some(next) = decoded
            .first()
            .and_then(|instruction| instruction.as_ref())
            .and_then(|instruction| entry_thunk_target(target, instruction))
        {
            target = next;
            continue;
        }
        let extent = function_extent(&decoded);
        let candidates = find_candidates(elf, segments, toc, target, &decoded[..=extent]);
        let mut gated = None;
        let mut saw_ungated = false;
        for candidate in candidates {
            match candidate {
                Candidate::Ungated => saw_ungated = true,
                Candidate::Gated { mask, fail_errno } => match gated {
                    None => gated = Some((mask, fail_errno)),
                    Some(existing) if existing == (mask, fail_errno) => {}
                    Some(_) => return Lv2Gate::NotAnalysed,
                },
            }
        }
        return match gated {
            Some((mask, fail_errno)) => Lv2Gate::Gated {
                reads: Lv2GateRead::ControlFlags1(mask),
                fail_errno,
            },
            None if saw_ungated => Lv2Gate::Ungated,
            None => Lv2Gate::NotAnalysed,
        };
    }
    Lv2Gate::NotAnalysed
}

fn entry_thunk_target(base: u64, instruction: &PpuInstruction) -> Option<u64> {
    let PpuInstruction::B {
        offset,
        aa,
        link: false,
    } = instruction
    else {
        return None;
    };
    if *aa {
        u64::try_from(i64::from(*offset)).ok()
    } else {
        add_signed(base, i64::from(*offset))
    }
}

fn find_candidates(
    elf: &[u8],
    segments: &[LoadSegment],
    toc: u64,
    base: u64,
    decoded: &[Option<PpuInstruction>],
) -> Vec<Candidate> {
    let mut candidates = Vec::new();
    for (call_index, instruction) in decoded.iter().enumerate() {
        if !matches!(instruction, Some(PpuInstruction::B { link: true, .. })) {
            continue;
        }
        let Some(record_offset) = recent_record_load(decoded, call_index) else {
            continue;
        };
        let end = (call_index + 9).min(decoded.len());
        let window = &decoded[call_index + 1..end];
        let Some((result_register, compare_index, compare_field)) = boolean_result(window) else {
            continue;
        };
        let Some((branch_after_compare, branch)) =
            branch_on_compare(&window[compare_index + 1..], compare_field)
        else {
            continue;
        };
        let Some(record_pointer_address) = add_signed(toc, i64::from(record_offset)) else {
            continue;
        };
        let Some(record) = read_u64_at(elf, segments, record_pointer_address) else {
            continue;
        };
        let Some(mask) = permission_record_mask(elf, segments, record) else {
            continue;
        };
        if mask == 0 {
            candidates.push(Candidate::Ungated);
            continue;
        }
        let search_start = call_index.saturating_sub(3);
        let search_end = (call_index + compare_index + 8).min(decoded.len());
        let Some((fail_errno, error_register)) = known_errno(&decoded[search_start..search_end])
        else {
            continue;
        };
        let branch_index = call_index + 1 + compare_index + 1 + branch_after_compare;
        if !error_path_returns(decoded, base, branch_index, branch, error_register) {
            continue;
        }
        let _ = result_register;
        candidates.push(Candidate::Gated { mask, fail_errno });
    }
    candidates
}

fn recent_record_load(decoded: &[Option<PpuInstruction>], call_index: usize) -> Option<i16> {
    decoded[..call_index]
        .iter()
        .rev()
        .take(16)
        .find_map(|instruction| match instruction {
            Some(PpuInstruction::Ld { rt: 4, ra: 2, imm }) => Some(*imm),
            _ => None,
        })
}

fn boolean_result(window: &[Option<PpuInstruction>]) -> Option<(u8, usize, u8)> {
    for (mask_index, instruction) in window.iter().enumerate() {
        let result = match instruction {
            Some(PpuInstruction::Rlwinm {
                ra,
                rs: 3,
                sh: 0,
                mb: 24,
                me: 31,
                rc: false,
            }) => *ra,
            Some(PpuInstruction::Rldicl {
                ra,
                rs: 3,
                sh: 0,
                mb: 56,
                rc: false,
            }) => *ra,
            _ => continue,
        };
        for (compare_index, compare) in window.iter().enumerate().skip(mask_index + 1) {
            let field = match compare {
                Some(PpuInstruction::Cmpwi { bf, ra, imm: 0 })
                | Some(PpuInstruction::Cmpdi { bf, ra, imm: 0 })
                    if *ra == result =>
                {
                    *bf
                }
                _ => continue,
            };
            return Some((result, compare_index, field));
        }
    }
    None
}

fn branch_on_compare(
    window: &[Option<PpuInstruction>],
    field: u8,
) -> Option<(usize, PpuInstruction)> {
    window
        .iter()
        .take(4)
        .enumerate()
        .find_map(|(index, instruction)| match instruction {
            Some(
                branch @ PpuInstruction::Bc {
                    bi, link: false, ..
                },
            ) if *bi / 4 == field => Some((index, *branch)),
            _ => None,
        })
}

fn known_errno(decoded: &[Option<PpuInstruction>]) -> Option<(u32, u8)> {
    for (index, instruction) in decoded.iter().enumerate() {
        let Some(PpuInstruction::Addis {
            rt,
            ra: 0,
            imm: high,
        }) = instruction
        else {
            continue;
        };
        for candidate in decoded.iter().skip(index + 1).take(6) {
            let Some(PpuInstruction::Ori { ra, rs, imm: low }) = candidate else {
                continue;
            };
            if ra != rt || rs != rt {
                continue;
            }
            let value = (u32::from(*high as u16) << 16) | u32::from(*low);
            if errno::lookup(value).is_some() {
                return Some((value, *rt));
            }
        }
    }
    None
}

fn error_path_returns(
    decoded: &[Option<PpuInstruction>],
    base: u64,
    branch_index: usize,
    branch: PpuInstruction,
    error_register: u8,
) -> bool {
    let PpuInstruction::Bc {
        offset,
        aa,
        link: false,
        ..
    } = branch
    else {
        return false;
    };
    let instruction_address = match base.checked_add((branch_index as u64).saturating_mul(4)) {
        Some(address) => address,
        None => return false,
    };
    let branch_target = if aa {
        u64::try_from(i64::from(offset)).ok()
    } else {
        add_signed(instruction_address, i64::from(offset))
    }
    .and_then(|address| address.checked_sub(base))
    .filter(|relative| relative % 4 == 0)
    .and_then(|relative| usize::try_from(relative / 4).ok());
    [Some(branch_index + 1), branch_target]
        .into_iter()
        .flatten()
        .any(|start| straight_path_returns(decoded, base, start, error_register))
}

fn straight_path_returns(
    decoded: &[Option<PpuInstruction>],
    base: u64,
    start: usize,
    error_register: u8,
) -> bool {
    let mut index = start;
    let mut result_ready = error_register == 3;
    for _ in 0..32 {
        let Some(instruction) = decoded.get(index).copied().flatten() else {
            return false;
        };
        match instruction {
            PpuInstruction::Extsw { ra: 3, rs, .. } if rs == error_register => {
                result_ready = true;
            }
            PpuInstruction::Or {
                ra: 3,
                rs,
                rb,
                rc: false,
            } if rs == error_register && rb == error_register => result_ready = true,
            PpuInstruction::Bclr {
                bo, link: false, ..
            } if bo & 0b10100 == 0b10100 => return result_ready,
            PpuInstruction::B {
                offset,
                aa,
                link: false,
            } => {
                let instruction_address = match base.checked_add((index as u64).saturating_mul(4)) {
                    Some(address) => address,
                    None => return false,
                };
                let target = if aa {
                    u64::try_from(i64::from(offset)).ok()
                } else {
                    add_signed(instruction_address, i64::from(offset))
                };
                let Some(next) = target
                    .and_then(|address| address.checked_sub(base))
                    .filter(|relative| relative % 4 == 0)
                    .and_then(|relative| usize::try_from(relative / 4).ok())
                else {
                    return false;
                };
                index = next;
                continue;
            }
            PpuInstruction::Bc { .. }
            | PpuInstruction::Bcctr { .. }
            | PpuInstruction::Bclr { .. } => return false,
            _ => {}
        }
        index += 1;
    }
    false
}

fn permission_record_mask(elf: &[u8], segments: &[LoadSegment], address: u64) -> Option<u32> {
    let offset = file_offset_at(segments, address, PERMISSION_RECORD_BYTES)?;
    let record = elf.get(offset..offset + PERMISSION_RECORD_BYTES)?;
    let mask = u32::from_be_bytes(record[..4].try_into().ok()?);
    record[4..].iter().all(|byte| *byte == 0).then_some(mask)
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

fn add_signed(base: u64, displacement: i64) -> Option<u64> {
    if displacement >= 0 {
        base.checked_add(displacement as u64)
    } else {
        base.checked_sub(displacement.unsigned_abs())
    }
}

#[cfg(test)]
#[path = "tests/lv2_gate_tests.rs"]
mod tests;
