//! The inputs `cellgov_compare::classify` reads: the non-semantic
//! guest ranges a title's EBOOT and a CellGov observation locate, and
//! one class per byte divergence.
//!
//! `cellgov_compare` reads no ELF, so this crate, which depends on the
//! PPU loader, locates the ranges.

use std::ops::Range;

use cellgov_compare::{
    classify, sync_primitive_scan, ClassifierContext, DivergenceClass, Observation,
    ObservationCompareResult, RegionPairOutcome, CODE_REGION_NAME, ELF_HEADER_SIZE,
};
use cellgov_ppu::loader::{self, LoadError};
use cellgov_ppu::prx::{self, ImportParseError};

/// Why the classifier context could not be built.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClassifierContextError {
    /// The EBOOT's header or program-header table is malformed.
    #[error("ELF header: {0}")]
    ElfHeader(#[from] LoadError),
    /// The EBOOT's import tables are malformed.
    #[error("parse imports: {0}")]
    ImportParse(#[from] ImportParseError),
    /// `addr + phdr_end` overflows `u64`. Reachable from an
    /// observation JSON naming a code region near `u64::MAX`.
    #[error("code region addr 0x{addr:016x} + PHDR-table end 0x{phdr_end:016x} overflows u64")]
    CodeRegionAddrOverflow {
        /// The code region's guest address.
        addr: u64,
        /// One past the header and program-header table.
        phdr_end: u64,
    },
    /// The `sys_process_param_t` record's end overflows `u64`.
    #[error("sys_process_param addr 0x{addr:016x} + struct_size {struct_size} overflows u64")]
    SysProcParamAddrOverflow {
        /// The record's guest address.
        addr: u64,
        /// The record's declared size.
        struct_size: u64,
    },
}

/// Build a [`ClassifierContext`] from EBOOT bytes and the CellGov
/// observation.
///
/// # Errors
///
/// [`ClassifierContextError::ElfHeader`] and
/// [`ClassifierContextError::ImportParse`] on a malformed EBOOT, and
/// the two overflow variants on addresses near `u64::MAX`. The header
/// is read only when the observation carries a code region.
pub fn build_classifier_context(
    eboot_bytes: &[u8],
    observation: &Observation,
) -> Result<ClassifierContext, ClassifierContextError> {
    let elf_header_range = observation
        .memory_regions
        .iter()
        .find(|r| r.name == CODE_REGION_NAME)
        .map(|r| -> Result<Range<u64>, ClassifierContextError> {
            let phdr_end = header_and_phdr_table_end(eboot_bytes)?;
            let end = r.addr.checked_add(phdr_end).ok_or(
                ClassifierContextError::CodeRegionAddrOverflow {
                    addr: r.addr,
                    phdr_end,
                },
            )?;
            Ok(r.addr..end)
        })
        .transpose()?;

    let sys_proc_param_range = match loader::find_sys_process_param(eboot_bytes) {
        Some(p) => {
            let size = p.struct_size as u64;
            let end = p.guest_addr.checked_add(size).ok_or(
                ClassifierContextError::SysProcParamAddrOverflow {
                    addr: p.guest_addr,
                    struct_size: size,
                },
            )?;
            Some(p.guest_addr..end)
        }
        None => None,
    };

    let hle_opd_ranges = hle_opd_ranges(eboot_bytes)?;

    // sys_lwmutex_t handle-slot scan runs on the runtime snapshot (not
    // the EBOOT) because the lwmutex_free sentinel and attribute field
    // are only populated post-init. Every captured region is walked: a
    // title with several writable PT_LOADs keeps its lwmutexes in
    // whichever one the linker chose; the preamble match guards
    // against false positives.
    let mut sync_primitive_id_ranges: Vec<Range<u64>> = observation
        .memory_regions
        .iter()
        .flat_map(|r| sync_primitive_scan::find_sys_lwmutex_handle_slots(&r.data, r.addr))
        .collect();
    // An lwcond names the lwmutex it binds, so its slots are found
    // against the lwmutex set of the same snapshot.
    let lwcond_slots: Vec<Range<u64>> = observation
        .memory_regions
        .iter()
        .flat_map(|r| {
            sync_primitive_scan::find_sys_lwcond_handle_slots(
                &r.data,
                r.addr,
                &sync_primitive_id_ranges,
            )
        })
        .collect();
    sync_primitive_id_ranges.extend(lwcond_slots);

    let ctx = ClassifierContext {
        elf_header_range,
        sys_proc_param_range,
        hle_opd_ranges,
        sync_primitive_id_ranges,
    };
    ctx.debug_assert_disjoint();
    Ok(ctx)
}

/// One past the end of the ELF header and the program-header table, as
/// a file offset, and at least [`ELF_HEADER_SIZE`]. The loader places
/// both at the start of the code segment.
///
/// The value becomes a classifier range that marks every divergent
/// byte under it non-semantic, so a table the file does not hold is
/// refused rather than trusted.
///
/// # Errors
///
/// Any [`loader::program_header_table`] refusal.
pub(crate) fn header_and_phdr_table_end(eboot_bytes: &[u8]) -> Result<u64, LoadError> {
    let table = loader::program_header_table(eboot_bytes)?;
    Ok(table.end().max(ELF_HEADER_SIZE as u64))
}

/// HLE-OPD-class slot ranges in the title's binary: one merged
/// range per maximal run of adjacent function-stub addresses, plus
/// one 4-byte range per variable-import `vref_addr`.
///
/// # Errors
///
/// [`ClassifierContextError::ImportParse`] if `parse_imports` rejects
/// the EBOOT. A parseable EBOOT that legitimately imports nothing
/// returns `NoImportsTable`, which this function maps to an empty vec
/// rather than an error.
pub(crate) fn hle_opd_ranges(
    eboot_bytes: &[u8],
) -> Result<Vec<Range<u64>>, ClassifierContextError> {
    let modules = match prx::parse_imports(eboot_bytes) {
        Ok(m) => m,
        Err(ImportParseError::NoImportsTable) => return Ok(Vec::new()),
        Err(e) => return Err(ClassifierContextError::ImportParse(e)),
    };

    let mut stubs: Vec<u32> = modules
        .iter()
        .flat_map(|m| m.functions.iter().map(|f| f.stub_addr))
        .collect();
    let mut ranges = merge_adjacent_stub_ranges(&mut stubs);

    let mut var_addrs: Vec<u32> = modules
        .iter()
        .flat_map(|m| m.variables.iter().map(|v| v.vref_addr))
        .collect();
    var_addrs.sort_unstable();
    var_addrs.dedup();
    for addr in var_addrs {
        // u32 cast bounds the arithmetic: u32::MAX + 4 fits in u64.
        debug_assert!((addr as u64).checked_add(4).is_some());
        ranges.push(addr as u64..addr as u64 + 4);
    }

    // Secondary OPD tables: adjacent tables collapse into one Range,
    // non-adjacent stay separate. Scan in
    // `cellgov_ppu::loader::find_secondary_opd_tables`.
    let secondary: Vec<Range<u64>> = loader::find_secondary_opd_tables(eboot_bytes)
        .into_iter()
        .map(|t| t.guest_addr..t.guest_addr + t.size)
        .collect();
    let mut merged: Option<Range<u64>> = None;
    for r in secondary {
        merged = Some(match merged {
            Some(cur) if cur.end == r.start => cur.start..r.end,
            Some(cur) => {
                ranges.push(cur);
                r
            }
            None => r,
        });
    }
    if let Some(r) = merged {
        ranges.push(r);
    }

    // Indirect OPD tables (12-byte (id, ptr, opd_slot) rows): each
    // table contributes its OPD slot at row offset
    // INDIRECT_OPD_TABLE_SLOT_OFFSET. Scan in
    // `cellgov_ppu::loader::find_indirect_opd_tables`.
    for table in loader::find_indirect_opd_tables(eboot_bytes) {
        let row_count = table.size / loader::INDIRECT_OPD_TABLE_STRIDE;
        for row in 0..row_count {
            let slot_start = table.guest_addr
                + row * loader::INDIRECT_OPD_TABLE_STRIDE
                + loader::INDIRECT_OPD_TABLE_SLOT_OFFSET;
            ranges.push(slot_start..slot_start + 4);
        }
    }

    Ok(ranges)
}

/// Sort, dedup, and merge 4-byte stub addresses into the smallest
/// set of non-overlapping ranges; abutting stubs merge.
pub(crate) fn merge_adjacent_stub_ranges(stubs: &mut Vec<u32>) -> Vec<Range<u64>> {
    stubs.sort_unstable();
    stubs.dedup();
    let mut ranges = Vec::new();
    let mut cur: Option<Range<u64>> = None;
    for s in stubs.iter() {
        debug_assert!((*s as u64).checked_add(4).is_some());
        let next = *s as u64..*s as u64 + 4;
        cur = Some(match cur {
            Some(r) if r.end == next.start => r.start..next.end,
            Some(r) => {
                ranges.push(r);
                next
            }
            None => next,
        });
    }
    if let Some(r) = cur {
        ranges.push(r);
    }
    ranges
}

/// One class per [`cellgov_compare::ByteDivergence`] in `result`,
/// in flatten order over regions and bytes-within-region.
///
/// `cellgov` must be the observation that seeded `ctx` via
/// [`build_classifier_context`]; `rpcs3` supplies the second image
/// the HleOpdSlot structural checks read.
pub fn classify_all(
    result: &ObservationCompareResult,
    cellgov: &Observation,
    rpcs3: &Observation,
    ctx: &ClassifierContext,
) -> Vec<DivergenceClass> {
    let mut classes = Vec::new();
    for pair in &result.region_compare.pairs {
        if let RegionPairOutcome::ByteDivergence {
            name, addr, bytes, ..
        } = pair
        {
            let a_region = cellgov.memory_regions.iter().find(|r| &r.name == name);
            debug_assert_eq!(
                a_region.map(|r| r.addr),
                Some(*addr),
                "ByteDivergence pair addr disagrees with cellgov observation; \
                 compare_observations IdentityMismatch invariant violated"
            );
            // A ByteDivergence pair exists only when both observations
            // carry the region; a miss here is the same broken-invariant
            // class as the addr mismatch above. The empty-slice fallback
            // fails closed (the slot checks refuse and the run lands in
            // Pending), but it must still announce itself in debug.
            let b_region = rpcs3.memory_regions.iter().find(|r| &r.name == name);
            debug_assert!(
                a_region.is_some() && b_region.is_some(),
                "ByteDivergence pair region {name:?} missing from an observation; \
                 compare_observations invariant violated"
            );
            let a_data: &[u8] = a_region.map(|r| r.data.as_slice()).unwrap_or(&[]);
            let b_data: &[u8] = b_region.map(|r| r.data.as_slice()).unwrap_or(&[]);
            for div in bytes {
                classes.push(classify(div, *addr, ctx, a_data, b_data));
            }
        }
    }
    classes
}

#[cfg(test)]
#[path = "tests/classifier_context_tests.rs"]
mod tests;
