//! Discovers the LV2 syscall dispatch table in a decrypted kernel ELF.

use std::collections::BTreeSet;

use cellgov_ps3_abi::format::elf::ppc64_function_descriptor;
use cellgov_ps3_abi::hw::ppu::SYSTEM_CALL_VECTOR_OFFSET;

use crate::instruction::PpuInstruction;
use crate::loader::{pt_load_segments, LoadError, LoadSegment};

const VECTOR_WORDS: usize = 64;
const HANDLER_WORDS: usize = 512;
const MAX_DISCOVERED_SLOTS: usize = 1 << 16;

/// Names the evidence path that produced a table discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2DiscoveryMethod {
    /// Uses the `sc` vector's index shape to find one descriptor-pointer array.
    ScVectorDescriptorArray,
}

impl Lv2DiscoveryMethod {
    /// Returns the stable label used in reports and archive rows.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ScVectorDescriptorArray => "sc_vector_descriptor_array",
        }
    }
}

/// States whether structural evidence uniquely identifies one table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2DiscoveryConfidence {
    /// Every structural check uniquely identifies the same table.
    High,
}

impl Lv2DiscoveryConfidence {
    /// Returns the stable label used in reports and archive rows.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
        }
    }
}

/// Describes how one dispatch-table entry names its handler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lv2TableEntryFormat {
    /// Stores a 64-bit pointer to a three-doubleword PPC64 function descriptor.
    Ppc64DescriptorPointer,
}

impl Lv2TableEntryFormat {
    /// Returns the stable label used in reports and archive rows.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ppc64DescriptorPointer => "ppc64_descriptor_pointer",
        }
    }
}

/// Records structural checks that support one discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv2DiscoveryEvidence {
    /// Counts executable addresses that the `sc` vector materializes.
    pub vector_targets: usize,
    /// Counts handler targets whose code contains the index sequence.
    pub handler_matches: usize,
    /// Counts table-shaped arrays that pass every descriptor check.
    pub table_candidates: usize,
    /// Counts table entries that name valid descriptors.
    pub descriptor_entries: usize,
    /// Counts distinct descriptors that the table names.
    pub unique_descriptors: usize,
    /// Counts entries that point to the entry-zero descriptor.
    pub entry_zero_references: usize,
    /// Reports whether the final slot points to the entry-zero descriptor.
    pub last_entry_is_entry_zero: bool,
    /// Counts descriptor entries with a zero environment word.
    pub zero_environments: usize,
    /// Reports whether every descriptor carries the same TOC.
    pub consistent_toc: bool,
    /// Reports whether the first word after the table is zero.
    ///
    /// `None` means that the table ends at the file-backed segment boundary.
    pub post_table_zero: Option<bool>,
    /// Contains the low 32 bits that entry zero returns when it is a three-instruction leaf.
    pub entry_zero_return: Option<u32>,
}

/// Describes one high-confidence LV2 dispatch-table discovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Lv2TableDiscovery {
    /// Names the discovery method.
    pub method: Lv2DiscoveryMethod,
    /// Gives the confidence level.
    pub confidence: Lv2DiscoveryConfidence,
    /// Gives the effective address of the architectural System Call vector.
    pub vector_vaddr: u64,
    /// Gives the effective address of the handler that contains the table index.
    pub handler_vaddr: u64,
    /// Gives the effective address of the table.
    pub table_vaddr: u64,
    /// Gives the file offset of the table inside the ELF.
    pub table_file_offset: u64,
    /// Gives the table entry count recovered from the handler bound.
    pub entry_count: usize,
    /// Gives the entry width recovered from the handler shift.
    pub entry_width: usize,
    /// Names the table entry representation.
    pub entry_format: Lv2TableEntryFormat,
    /// Gives the TOC that every referenced function descriptor shares.
    pub toc: u64,
    /// Evidence that supports the confidence.
    pub evidence: Lv2DiscoveryEvidence,
}

/// Reports why an ELF did not produce one unique, high-confidence table.
#[derive(Debug, thiserror::Error)]
pub enum Lv2TableDiscoveryError {
    /// The ELF has malformed load segments.
    #[error("LV2 table discovery: {0}")]
    Elf(#[from] LoadError),
    /// A load segment lies outside the input or violates ELF bounds.
    #[error("LV2 table discovery: malformed PT_LOAD segment {index}")]
    MalformedSegment {
        /// Gives the program-header index.
        index: usize,
    },
    /// The ELF has no executable load segment.
    #[error("LV2 table discovery: ELF has no executable PT_LOAD")]
    NoExecutableSegment,
    /// The System Call vector is absent or names no executable handler.
    #[error("LV2 table discovery: System Call vector yields no indexed handler")]
    HandlerNotFound,
    /// Several plausible table-index sequences remain.
    #[error("LV2 table discovery: {count} plausible handler index sequences remain")]
    AmbiguousHandler {
        /// Gives the plausible sequence count.
        count: usize,
    },
    /// No descriptor-pointer array matches the handler's bound and stride.
    #[error(
        "LV2 table discovery: no table matches handler shape count={entry_count} width={entry_width}"
    )]
    NoTableCandidate {
        /// Gives the entry count recovered from the handler.
        entry_count: usize,
        /// Gives the entry width recovered from the handler.
        entry_width: usize,
    },
    /// Several arrays satisfy every structural check.
    #[error("LV2 table discovery: {count} table candidates remain; refusing ambiguity")]
    AmbiguousTable {
        /// Gives the fully valid candidate count.
        count: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IndexShape {
    handler_vaddr: u64,
    entry_count: usize,
    entry_width: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TableCandidate {
    table_vaddr: u64,
    table_file_offset: u64,
    toc: u64,
    unique_descriptors: usize,
    entry_zero_return: Option<u32>,
    entry_zero_references: usize,
    post_table_zero: Option<bool>,
}

/// Discover the LV2 dispatch table in a decrypted kernel ELF.
///
/// Discovery returns only a unique, high-confidence table.
///
/// # Errors
///
/// Returns [`Lv2TableDiscoveryError`] if discovery cannot produce one unique,
/// high-confidence table.
pub fn discover(elf: &[u8]) -> Result<Lv2TableDiscovery, Lv2TableDiscoveryError> {
    let segments = pt_load_segments(elf)?;
    validate_segments(elf, &segments)?;
    let image_base = segments
        .iter()
        .filter(|segment| segment.executable)
        .map(|segment| segment.vaddr)
        .min()
        .ok_or(Lv2TableDiscoveryError::NoExecutableSegment)?;
    let vector_vaddr = image_base
        .checked_add(SYSTEM_CALL_VECTOR_OFFSET)
        .ok_or(Lv2TableDiscoveryError::HandlerNotFound)?;
    let vector_words = words_at(elf, &segments, vector_vaddr, VECTOR_WORDS)
        .ok_or(Lv2TableDiscoveryError::HandlerNotFound)?;
    let vector_targets: Vec<_> = materialized_addresses(&vector_words)
        .into_iter()
        .filter(|target| is_executable_address(&segments, *target))
        .collect();

    let mut shapes = Vec::new();
    for target in &vector_targets {
        let Some(words) = words_at(elf, &segments, *target, HANDLER_WORDS) else {
            continue;
        };
        for (entry_count, entry_width) in index_shapes(&words, *target) {
            shapes.push(IndexShape {
                handler_vaddr: *target,
                entry_count,
                entry_width,
            });
        }
    }
    shapes.sort_by_key(|shape| (shape.handler_vaddr, shape.entry_count, shape.entry_width));
    let shape = match shapes.as_slice() {
        [] => return Err(Lv2TableDiscoveryError::HandlerNotFound),
        [shape] => *shape,
        many => {
            return Err(Lv2TableDiscoveryError::AmbiguousHandler { count: many.len() });
        }
    };

    let candidates = scan_candidates(elf, &segments, shape);
    let candidate = match candidates.as_slice() {
        [] => {
            return Err(Lv2TableDiscoveryError::NoTableCandidate {
                entry_count: shape.entry_count,
                entry_width: shape.entry_width,
            });
        }
        [candidate] => *candidate,
        many => {
            return Err(Lv2TableDiscoveryError::AmbiguousTable { count: many.len() });
        }
    };

    Ok(Lv2TableDiscovery {
        method: Lv2DiscoveryMethod::ScVectorDescriptorArray,
        confidence: Lv2DiscoveryConfidence::High,
        vector_vaddr,
        handler_vaddr: shape.handler_vaddr,
        table_vaddr: candidate.table_vaddr,
        table_file_offset: candidate.table_file_offset,
        entry_count: shape.entry_count,
        entry_width: shape.entry_width,
        entry_format: Lv2TableEntryFormat::Ppc64DescriptorPointer,
        toc: candidate.toc,
        evidence: Lv2DiscoveryEvidence {
            vector_targets: vector_targets.len(),
            handler_matches: shapes.len(),
            table_candidates: candidates.len(),
            descriptor_entries: shape.entry_count,
            unique_descriptors: candidate.unique_descriptors,
            entry_zero_references: candidate.entry_zero_references,
            last_entry_is_entry_zero: true,
            zero_environments: shape.entry_count,
            consistent_toc: true,
            post_table_zero: candidate.post_table_zero,
            entry_zero_return: candidate.entry_zero_return,
        },
    })
}

fn validate_segments(elf: &[u8], segments: &[LoadSegment]) -> Result<(), Lv2TableDiscoveryError> {
    for segment in segments {
        if segment.filesz > segment.memsz
            || segment
                .file_offset
                .checked_add(segment.filesz)
                .is_none_or(|end| end > elf.len() as u64)
            || segment.vaddr.checked_add(segment.memsz).is_none()
        {
            return Err(Lv2TableDiscoveryError::MalformedSegment {
                index: segment.index,
            });
        }
    }
    Ok(())
}

fn materialized_addresses(words: &[u32]) -> Vec<u64> {
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

fn index_shapes(words: &[u32], base: u64) -> Vec<(usize, usize)> {
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

fn scan_candidates(elf: &[u8], segments: &[LoadSegment], shape: IndexShape) -> Vec<TableCandidate> {
    let Some(table_bytes) = shape.entry_count.checked_mul(shape.entry_width) else {
        return Vec::new();
    };
    if shape.entry_width != core::mem::size_of::<u64>() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for segment in segments.iter().filter(|segment| segment.readable) {
        let Ok(filesz) = usize::try_from(segment.filesz) else {
            continue;
        };
        let Some(limit) = filesz.checked_sub(table_bytes) else {
            continue;
        };
        let alignment = shape.entry_width as u64;
        let skip = ((alignment - segment.vaddr % alignment) % alignment) as usize;
        for relative in (skip..=limit).step_by(shape.entry_width) {
            let table_vaddr = segment.vaddr + relative as u64;
            let post = read_u64_at(elf, segments, table_vaddr + table_bytes as u64);
            let Some(first_ptr) = read_u64_at(elf, segments, table_vaddr) else {
                continue;
            };
            let Some(first) = descriptor_at(elf, segments, first_ptr) else {
                continue;
            };
            let Some(entry_zero_return) = constant_return(elf, segments, first.code) else {
                continue;
            };
            let mut unique = BTreeSet::new();
            unique.insert(first_ptr);
            let mut entry_zero_references = 1usize;
            let mut last_pointer = first_ptr;
            let mut valid = true;
            for index in 1..shape.entry_count {
                let Some(address) = table_vaddr.checked_add((index * shape.entry_width) as u64)
                else {
                    valid = false;
                    break;
                };
                let Some(pointer) = read_u64_at(elf, segments, address) else {
                    valid = false;
                    break;
                };
                let Some(descriptor) = descriptor_at(elf, segments, pointer) else {
                    valid = false;
                    break;
                };
                if descriptor.toc != first.toc || descriptor.env != 0 {
                    valid = false;
                    break;
                }
                if pointer == first_ptr {
                    entry_zero_references += 1;
                }
                last_pointer = pointer;
                unique.insert(pointer);
            }
            if !valid || first.env != 0 || last_pointer != first_ptr {
                continue;
            }
            let Some(file_offset) = file_offset_at(segments, table_vaddr, table_bytes) else {
                continue;
            };
            out.push(TableCandidate {
                table_vaddr,
                table_file_offset: file_offset as u64,
                toc: first.toc,
                unique_descriptors: unique.len(),
                entry_zero_return: Some(entry_zero_return),
                entry_zero_references,
                post_table_zero: post.map(|word| word == 0),
            });
        }
    }
    out
}

#[derive(Debug, Clone, Copy)]
struct Descriptor {
    code: u64,
    toc: u64,
    env: u64,
}

fn descriptor_at(elf: &[u8], segments: &[LoadSegment], address: u64) -> Option<Descriptor> {
    if !address.is_multiple_of(core::mem::size_of::<u64>() as u64) {
        return None;
    }
    let offset = file_offset_at(segments, address, ppc64_function_descriptor::SIZE)?;
    let code = read_be_u64(elf, offset + ppc64_function_descriptor::CODE_OFFSET)?;
    let toc = read_be_u64(elf, offset + ppc64_function_descriptor::TOC_OFFSET)?;
    let env = read_be_u64(elf, offset + ppc64_function_descriptor::ENV_OFFSET)?;
    if code % 4 != 0 || !is_executable_address(segments, code) || !is_mapped_address(segments, toc)
    {
        return None;
    }
    Some(Descriptor { code, toc, env })
}

fn constant_return(elf: &[u8], segments: &[LoadSegment], address: u64) -> Option<u32> {
    let words = words_at(elf, segments, address, 3)?;
    let Ok(PpuInstruction::Addis {
        rt: 3,
        ra: 0,
        imm: high,
    }) = crate::decode::decode(words[0])
    else {
        return None;
    };
    let Ok(PpuInstruction::Ori {
        ra: 3,
        rs: 3,
        imm: low,
    }) = crate::decode::decode(words[1])
    else {
        return None;
    };
    if !matches!(
        crate::decode::decode(words[2]),
        // [PPC-Book1 p:20 s:2.4.1] BO=1z1zz is unconditional.
        Ok(PpuInstruction::Bclr { bo, link: false, .. }) if bo & 0b10100 == 0b10100
    ) {
        return None;
    }
    Some((u32::from(high as u16) << 16) | u32::from(low))
}

fn words_at(
    elf: &[u8],
    segments: &[LoadSegment],
    address: u64,
    max_words: usize,
) -> Option<Vec<u32>> {
    let segment = segments.iter().find(|segment| {
        segment.executable
            && address >= segment.vaddr
            && address < segment.vaddr.saturating_add(segment.filesz)
    })?;
    let available = segment
        .vaddr
        .checked_add(segment.filesz)?
        .checked_sub(address)? as usize;
    let word_count = max_words.min(available / 4);
    let offset = file_offset_at(segments, address, word_count * 4)?;
    Some(
        elf[offset..offset + word_count * 4]
            .chunks_exact(4)
            .map(|bytes| u32::from_be_bytes(bytes.try_into().expect("four-byte chunk")))
            .collect(),
    )
}

fn read_u64_at(elf: &[u8], segments: &[LoadSegment], address: u64) -> Option<u64> {
    let offset = file_offset_at(segments, address, core::mem::size_of::<u64>())?;
    read_be_u64(elf, offset)
}

fn read_be_u64(bytes: &[u8], offset: usize) -> Option<u64> {
    let end = offset.checked_add(8)?;
    Some(u64::from_be_bytes(bytes.get(offset..end)?.try_into().ok()?))
}

fn file_offset_at(segments: &[LoadSegment], address: u64, len: usize) -> Option<usize> {
    let len = len as u64;
    for segment in segments {
        let Some(delta) = address.checked_sub(segment.vaddr) else {
            continue;
        };
        let Some(end) = delta.checked_add(len) else {
            continue;
        };
        if end > segment.filesz {
            continue;
        }
        let offset = segment.file_offset.checked_add(delta)?;
        return usize::try_from(offset).ok();
    }
    None
}

fn is_executable_address(segments: &[LoadSegment], address: u64) -> bool {
    segments.iter().any(|segment| {
        segment.executable
            && address >= segment.vaddr
            && address < segment.vaddr.saturating_add(segment.filesz)
    })
}

fn is_mapped_address(segments: &[LoadSegment], address: u64) -> bool {
    segments.iter().any(|segment| {
        address >= segment.vaddr && address < segment.vaddr.saturating_add(segment.memsz)
    })
}

#[cfg(test)]
#[path = "tests/lv2_table_tests.rs"]
mod tests;
