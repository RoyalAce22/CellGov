//! Code analysis: the addresses a handler materializes, its index shapes, the instructions it reaches, and constant-return stubs.

use crate::instruction::PpuInstruction;
use crate::loader::LoadSegment;

use super::discover::MAX_DISCOVERED_SLOTS;
use super::read::words_at;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct IndexShape {
    pub(super) handler_vaddr: u64,
    pub(super) entry_count: usize,
    pub(super) entry_width: usize,
}

pub(super) fn materialized_addresses(words: &[u32]) -> Vec<u64> {
    let mut out = Vec::new();
    for window in words.windows(5) {
        // [PPC-Book1 p:72 s:3.3] `sldi 32` is `rldicr 32, 31`.
        let [a, b, c, d, e] = window else {
            continue;
        };
        let (
            Ok(PpuInstruction::Addis { rt, ra: 0, imm: hi }),
            Ok(PpuInstruction::Ori {
                ra: mid_ra,
                rs: mid_rs,
                imm: mid,
            }),
            Ok(PpuInstruction::Rldicr {
                ra: shift_ra,
                rs: shift_rs,
                sh: 32,
                me: 31,
                ..
            }),
            Ok(PpuInstruction::Oris {
                ra: low_hi_ra,
                rs: low_hi_rs,
                imm: low_hi,
            }),
            Ok(PpuInstruction::Ori {
                ra: low_ra,
                rs: low_rs,
                imm: low,
            }),
        ) = (
            crate::decode::decode(*a),
            crate::decode::decode(*b),
            crate::decode::decode(*c),
            crate::decode::decode(*d),
            crate::decode::decode(*e),
        )
        else {
            continue;
        };
        if [
            mid_ra, mid_rs, shift_ra, shift_rs, low_hi_ra, low_hi_rs, low_ra, low_rs,
        ]
        .into_iter()
        .all(|register| register == rt)
        {
            out.push(
                (u64::from(hi as u16) << 48)
                    | (u64::from(mid) << 32)
                    | (u64::from(low_hi) << 16)
                    | u64::from(low),
            );
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

pub(super) fn index_shapes(words: &[u32], base: u64) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let reachable = reachable_instructions(words, base);
    for (start, window) in words.windows(10).enumerate() {
        if !reachable[start..start + 10].iter().all(|value| *value) {
            continue;
        }
        let Ok(PpuInstruction::Cmpldi {
            bf,
            ra: index,
            imm: bound,
        }) = crate::decode::decode(window[0])
        else {
            continue;
        };
        let decoded: Vec<_> = window[1..]
            .iter()
            .map(|word| crate::decode::decode(*word))
            .collect();
        // [PPC-Book1 p:20 s:2.4.1] BO=011at branches when the selected CR bit is one.
        let branches_on_unsigned_less = matches!(
            decoded[0],
            Ok(PpuInstruction::Bc {
                bo,
                bi,
                offset: 8,
                aa: false,
                link: false,
            }) if bo & 0b11100 == 0b01100
                && bo & 0b11 != 0b01
                && bi == bf * 4
        );
        if !branches_on_unsigned_less
            || !matches!(
                decoded[1],
                Ok(PpuInstruction::Addi {
                    rt,
                    ra: 0,
                    imm: 0
                }) if rt == index
            )
        {
            continue;
        }
        let Ok(PpuInstruction::Rldicr {
            ra: shift_ra,
            rs: shift_rs,
            sh,
            me,
            ..
        }) = decoded[2]
        else {
            continue;
        };
        // [PPC-Book1 p:160 s:B.7.1] `sldi n` is `rldicr n, 63-n`.
        if shift_ra != index || shift_rs != index || me != 63 - sh {
            continue;
        }
        let Ok(PpuInstruction::Add { rt, ra, rb, .. }) = decoded[3] else {
            continue;
        };
        let base = if (ra == index && rt == rb) || (rb == index && rt == ra) {
            rt
        } else {
            continue;
        };
        if !matches!(decoded[4], Ok(PpuInstruction::Ld { rt, ra, imm: 0 }) if rt == base && ra == base)
            || !matches!(decoded[5], Ok(PpuInstruction::Ld { rt, ra, imm: 0 }) if rt == base && ra == base)
        {
            continue;
        }
        let Some(mtlr_index) = decoded[6..].iter().position(
            |instruction| matches!(instruction, Ok(PpuInstruction::Mtlr { rs }) if *rs == base),
        ) else {
            continue;
        };
        if mtlr_index != 1
            || !matches!(decoded[6], Ok(PpuInstruction::Stdu { ra, .. }) if ra != base)
        {
            continue;
        }
        let branch_index = 6 + mtlr_index + 1;
        if branch_index >= decoded.len()
            || !matches!(
                decoded[branch_index],
                // [PPC-Book1 p:20 s:2.4.1] BO=1z1zz is unconditional.
                Ok(PpuInstruction::Bclr { bo, link: true, .. }) if bo & 0b10100 == 0b10100
            )
        {
            continue;
        }
        let entry_count = usize::from(bound);
        let Some(entry_width) = 1usize.checked_shl(u32::from(sh)) else {
            continue;
        };
        if entry_count != 0 && entry_count <= MAX_DISCOVERED_SLOTS {
            out.push((entry_count, entry_width));
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

fn reachable_instructions(words: &[u32], base: u64) -> Vec<bool> {
    let mut reachable = vec![false; words.len()];
    if words.is_empty() {
        return reachable;
    }
    let mut pending = vec![0usize];
    while let Some(index) = pending.pop() {
        if index >= words.len() || reachable[index] {
            continue;
        }
        reachable[index] = true;
        let decoded = crate::decode::decode(words[index]);
        match decoded {
            Ok(PpuInstruction::B {
                offset, aa, link, ..
            }) => {
                if link {
                    push_fallthrough(&mut pending, index, words.len());
                } else if let Some(target) =
                    local_branch_index(base, index, i64::from(offset), aa, words.len())
                {
                    pending.push(target);
                }
            }
            Ok(PpuInstruction::Bc {
                bo,
                offset,
                aa,
                link,
                ..
            }) => {
                if link || bo & 0b10100 != 0b10100 {
                    push_fallthrough(&mut pending, index, words.len());
                }
                if !link {
                    if let Some(target) =
                        local_branch_index(base, index, i64::from(offset), aa, words.len())
                    {
                        pending.push(target);
                    }
                }
            }
            Ok(PpuInstruction::Bclr { bo, link, .. })
            | Ok(PpuInstruction::Bcctr { bo, link, .. }) => {
                if link || bo & 0b10100 != 0b10100 {
                    push_fallthrough(&mut pending, index, words.len());
                }
            }
            _ => push_fallthrough(&mut pending, index, words.len()),
        }
    }
    reachable
}

fn push_fallthrough(pending: &mut Vec<usize>, index: usize, word_count: usize) {
    if index + 1 < word_count {
        pending.push(index + 1);
    }
}

fn local_branch_index(
    base: u64,
    index: usize,
    offset: i64,
    absolute: bool,
    word_count: usize,
) -> Option<usize> {
    let current = base.checked_add((index * 4) as u64)?;
    let target = if absolute {
        u64::try_from(offset).ok()?
    } else if offset >= 0 {
        current.checked_add(offset as u64)?
    } else {
        current.checked_sub(offset.unsigned_abs())?
    };
    let delta = target.checked_sub(base)?;
    if !delta.is_multiple_of(4) {
        return None;
    }
    let target_index = usize::try_from(delta / 4).ok()?;
    (target_index < word_count).then_some(target_index)
}

pub(crate) fn constant_return(elf: &[u8], segments: &[LoadSegment], address: u64) -> Option<u32> {
    let words = words_at(elf, segments, address, 3)?;
    let [first, second, third, ..] = words.as_slice() else {
        return None;
    };
    let Ok(PpuInstruction::Addis {
        rt: 3,
        ra: 0,
        imm: high,
    }) = crate::decode::decode(*first)
    else {
        return None;
    };
    let Ok(PpuInstruction::Ori {
        ra: 3,
        rs: 3,
        imm: low,
    }) = crate::decode::decode(*second)
    else {
        return None;
    };
    if !matches!(
        crate::decode::decode(*third),
        // [PPC-Book1 p:20 s:2.4.1] BO=1z1zz is unconditional.
        Ok(PpuInstruction::Bclr { bo, link: false, .. }) if bo & 0b10100 == 0b10100
    ) {
        return None;
    }
    Some((u32::from(high as u16) << 16) | u32::from(low))
}
