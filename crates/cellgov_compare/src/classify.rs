//! Divergence classifier: maps each [`ByteDivergence`] to a known
//! non-semantic class or `Unclassified`.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::observation::{Observation, CODE_REGION_NAME};
use crate::observation_compare::ByteDivergence;

pub use cellgov_ps3_abi::elf::ELF_HEADER_SIZE;

/// Classified shape of a single byte-divergence run.
///
/// Display strings are part of the on-disk fixture wire form
/// (`compare_report.txt`, `NOTES.md`); pinned by
/// `divergence_class_display_strings_are_stable`.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    thiserror::Error,
    strum::VariantArray,
)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceClass {
    /// Bytes inside the loaded ELF header. Non-semantic; the running
    /// program never reads them.
    #[error("ElfHeader")]
    ElfHeader,
    /// Bytes inside the `sys_process_param_t` struct's load location.
    /// Non-semantic; both runners read the parsed fields via internal
    /// state, not by re-reading the loaded bytes.
    #[error("SysProcParam")]
    SysProcParam,
    /// Bytes inside an HLE OPD trampoline slot. Non-semantic;
    /// per-slot pointer indices differ across runners but the
    /// resolved entry points are equivalent.
    #[error("HleOpdSlot")]
    HleOpdSlot,
    /// Bytes inside an LV2 sync-primitive user-side handle slot
    /// (e.g. `sys_lwmutex_t::sleep_queue`). The field carries an
    /// ABI-opaque kernel-allocated id consumed only through
    /// sync-primitive syscalls (`_sys_lwmutex_lock`,
    /// `sys_lwmutex_unlock`, etc.), which look the id up in the
    /// runner-local id table. Per-runner id values differ because
    /// the two kernels run independent allocators; every read of
    /// the field flows back to its owning kernel and resolves to
    /// the same logical sync object. The warrant is ABI-contract
    /// (handle is opaque to user code), so the populator must key
    /// only on slots whose layout proves the field is a kernel
    /// handle.
    #[error("SyncPrimitiveId")]
    SyncPrimitiveId,
    /// No populated context range contained this divergence run.
    /// Counted in the byte-parity Pending bucket and enumerated in
    /// `cross_runner_summary.json`'s `unclassified_runs`.
    #[error("Unclassified")]
    Unclassified,
}

impl DivergenceClass {
    /// True for classes whose bytes are known not to influence
    /// guest-observable behavior; the [`Unclassified`](Self::Unclassified)
    /// catch-all returns false.
    pub fn is_non_semantic(&self) -> bool {
        match self {
            Self::ElfHeader | Self::SysProcParam | Self::HleOpdSlot | Self::SyncPrimitiveId => true,
            Self::Unclassified => false,
        }
    }
}

/// Pre-computed guest-address ranges the classifier checks for
/// containment.
///
/// `hle_opd_ranges` aggregates three structurally distinct kinds,
/// all classifying as `HleOpdSlot`: the primary function-stub
/// table (one contiguous range from SCE PRX_PARAM
/// `lib_stub_start..lib_stub_end`), zero or more 4-byte
/// variable-stub slots, and zero or more contiguous secondary
/// tables identified by the `0x04020100` / `0x04020200` header
/// signature.
///
/// All entries must be pairwise non-overlapping; checked by
/// [`debug_assert_disjoint`](Self::debug_assert_disjoint).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassifierContext {
    /// Guest-address range of the loaded ELF header, or `None` until
    /// a populator slice supplies it.
    pub elf_header_range: Option<Range<u64>>,
    /// Guest-address range of the `sys_process_param_t` struct's load
    /// location, or `None` until a populator slice supplies it.
    pub sys_proc_param_range: Option<Range<u64>>,
    /// Guest-address ranges of HLE OPD trampoline slots (primary
    /// table, variable stubs, FNID-walker-patched sibling tables).
    /// Empty until a populator slice fills them in.
    pub hle_opd_ranges: Vec<Range<u64>>,
    /// Guest-address ranges of LV2 sync-primitive handle slots
    /// (currently the `sys_lwmutex_t::sleep_queue` field at +0x10
    /// of every `sys_lwmutex_t` in the title's data segment).
    /// Each range covers exactly the 4-byte handle field; the
    /// rest of the sync-primitive struct (lock_var, attribute,
    /// recursive_count, pad) is not claimed because it does not
    /// share the opaque-handle warrant. Empty until a populator
    /// slice fills it in.
    pub sync_primitive_id_ranges: Vec<Range<u64>>,
}

/// Errors from [`ClassifierContext::from_observation`]. Only fires
/// on synthetic / malformed observations; real PRX/ELF inputs
/// satisfy the size and overflow preconditions.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ClassifierContextError {
    /// Observation has no region named `CODE_REGION_NAME`.
    #[error("observation lacks the {CODE_REGION_NAME:?} region")]
    MissingCodeRegion,
    /// The `code` region carries fewer than `ELF_HEADER_SIZE` bytes,
    /// so an ELF-header range past end-of-data would be constructed.
    #[error("{CODE_REGION_NAME:?} region carries {len} bytes (< ELF_HEADER_SIZE = {needed})")]
    ShortCodeRegion {
        /// Actual region data length.
        len: usize,
        /// Required minimum (`ELF_HEADER_SIZE`).
        needed: usize,
    },
    /// `region.addr + ELF_HEADER_SIZE` overflows u64.
    #[error("code region addr 0x{addr:016x} + ELF_HEADER_SIZE overflows u64")]
    RegionEndOverflow {
        /// The region's base address.
        addr: u64,
    },
}

impl ClassifierContext {
    /// Build a context with only `elf_header_range` populated from
    /// the observation's `"code"` region; the fuller context is
    /// built from EBOOT bytes by real boots.
    ///
    /// # Errors
    ///
    /// Returns [`ClassifierContextError`] if the observation has no
    /// `"code"` region, that region is shorter than [`ELF_HEADER_SIZE`],
    /// or the region's address+ELF_HEADER_SIZE overflows.
    pub fn from_observation(obs: &Observation) -> Result<Self, ClassifierContextError> {
        let code = obs
            .memory_regions
            .iter()
            .find(|r| r.name == CODE_REGION_NAME)
            .ok_or(ClassifierContextError::MissingCodeRegion)?;
        if code.data.len() < ELF_HEADER_SIZE {
            return Err(ClassifierContextError::ShortCodeRegion {
                len: code.data.len(),
                needed: ELF_HEADER_SIZE,
            });
        }
        let end = code
            .addr
            .checked_add(ELF_HEADER_SIZE as u64)
            .ok_or(ClassifierContextError::RegionEndOverflow { addr: code.addr })?;
        let ctx = Self {
            elf_header_range: Some(code.addr..end),
            sys_proc_param_range: None,
            hle_opd_ranges: Vec::new(),
            sync_primitive_id_ranges: Vec::new(),
        };
        ctx.debug_assert_disjoint();
        Ok(ctx)
    }

    /// Panic in debug builds if any populated range is inverted
    /// (`start > end`) or if two populated ranges overlap.
    pub fn debug_assert_disjoint(&self) {
        #[cfg(debug_assertions)]
        {
            let mut all: Vec<(&str, &Range<u64>)> = Vec::new();
            if let Some(r) = &self.elf_header_range {
                all.push(("elf_header_range", r));
            }
            if let Some(r) = &self.sys_proc_param_range {
                all.push(("sys_proc_param_range", r));
            }
            for r in &self.hle_opd_ranges {
                all.push(("hle_opd_ranges", r));
            }
            for r in &self.sync_primitive_id_ranges {
                all.push(("sync_primitive_id_ranges", r));
            }
            for (n, r) in &all {
                assert!(
                    r.start <= r.end,
                    "ClassifierContext range {n} is inverted: {r:?}"
                );
            }
            for i in 0..all.len() {
                for j in (i + 1)..all.len() {
                    let (na, ra) = all[i];
                    let (nb, rb) = all[j];
                    assert!(
                        ra.end <= rb.start || rb.end <= ra.start,
                        "ClassifierContext ranges overlap: {na} {ra:?} vs {nb} {rb:?}"
                    );
                }
            }
        }
    }
}

/// Classify a single byte-divergence run by full containment in one
/// of `ctx`'s named ranges. Partial overlap returns `Unclassified`.
///
/// `region_addr` is the `addr` field of the [`NamedMemoryRegion`] the
/// divergence belongs to.
///
/// # Panics
///
/// In debug, on `div.length == 0` (see [`ByteDivergence::length`])
/// or guest-range arithmetic overflow.
///
/// [`NamedMemoryRegion`]: crate::observation::NamedMemoryRegion
pub fn classify(
    div: &ByteDivergence,
    region_addr: u64,
    ctx: &ClassifierContext,
) -> DivergenceClass {
    debug_assert!(div.length > 0, "ByteDivergence::length must be >= 1");
    let start = region_addr
        .checked_add(div.offset)
        .expect("region_addr + div.offset overflows u64");
    let end = start
        .checked_add(div.length)
        .expect("div range end overflows u64");

    for (range, class) in [
        (&ctx.elf_header_range, DivergenceClass::ElfHeader),
        (&ctx.sys_proc_param_range, DivergenceClass::SysProcParam),
    ] {
        if let Some(r) = range.as_ref() {
            if r.start <= start && end <= r.end {
                return class;
            }
        }
    }
    for r in &ctx.hle_opd_ranges {
        if r.start <= start && end <= r.end {
            return DivergenceClass::HleOpdSlot;
        }
    }
    for r in &ctx.sync_primitive_id_ranges {
        if r.start <= start && end <= r.end {
            return DivergenceClass::SyncPrimitiveId;
        }
    }
    DivergenceClass::Unclassified
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::{NamedMemoryRegion, ObservationMetadata, ObservedOutcome};
    use strum::VariantArray;

    #[test]
    fn divergence_class_variants_are_distinct_and_labels_are_unique() {
        let labels: std::collections::BTreeSet<String> = DivergenceClass::VARIANTS
            .iter()
            .map(|c| c.to_string())
            .collect();
        assert_eq!(
            labels.len(),
            DivergenceClass::VARIANTS.len(),
            "DivergenceClass label collision under to_string(): {labels:?}",
        );
    }

    fn region(name: &str, addr: u64, length: u64) -> NamedMemoryRegion {
        NamedMemoryRegion {
            name: name.to_string(),
            addr,
            data: vec![0u8; length as usize],
        }
    }

    fn div(offset: u64, length: u64) -> ByteDivergence {
        ByteDivergence {
            offset,
            length,
            a_byte: 0,
            b_byte: 1,
        }
    }

    fn obs_with(regions: Vec<NamedMemoryRegion>) -> Observation {
        Observation {
            outcome: ObservedOutcome::Completed,
            memory_regions: regions,
            events: Vec::new(),
            state_hashes: None,
            metadata: ObservationMetadata {
                runner: "cellgov".to_string(),
                steps: Some(1),
            },
            tty_log: Vec::new(),
        }
    }

    #[test]
    fn divergence_class_display_strings_are_stable() {
        assert_eq!(format!("{}", DivergenceClass::ElfHeader), "ElfHeader");
        assert_eq!(format!("{}", DivergenceClass::SysProcParam), "SysProcParam");
        assert_eq!(format!("{}", DivergenceClass::HleOpdSlot), "HleOpdSlot");
        assert_eq!(
            format!("{}", DivergenceClass::SyncPrimitiveId),
            "SyncPrimitiveId",
        );
        assert_eq!(format!("{}", DivergenceClass::Unclassified), "Unclassified");
    }

    #[test]
    fn divergence_class_is_non_semantic_for_four_known_classes() {
        assert!(DivergenceClass::ElfHeader.is_non_semantic());
        assert!(DivergenceClass::SysProcParam.is_non_semantic());
        assert!(DivergenceClass::HleOpdSlot.is_non_semantic());
        assert!(DivergenceClass::SyncPrimitiveId.is_non_semantic());
        assert!(!DivergenceClass::Unclassified.is_non_semantic());
    }

    #[test]
    fn sync_primitive_id_range_classifies_when_populated() {
        let r = region("data", 0x860000, 0x10000);
        let range: Range<u64> = 0x862000..0x862004;
        let ctx = ClassifierContext {
            sync_primitive_id_ranges: vec![range],
            ..ClassifierContext::default()
        };
        assert_eq!(
            classify(&div(0x2000, 4), r.addr, &ctx),
            DivergenceClass::SyncPrimitiveId,
        );
        assert_eq!(
            classify(&div(0x2004, 4), r.addr, &ctx),
            DivergenceClass::Unclassified,
        );
    }

    #[test]
    fn elf_header_offset_inside_range_classifies_as_elf_header() {
        let r = region("code", 0x10000, 0x40);
        let ctx = ClassifierContext {
            elf_header_range: Some(0x10000..0x10040),
            ..ClassifierContext::default()
        };
        let class = classify(&div(0x35, 1), r.addr, &ctx);
        assert_eq!(class, DivergenceClass::ElfHeader);
    }

    #[test]
    fn elf_header_offset_just_past_end_does_not_match() {
        let r = region("code", 0x10000, 0x40);
        let ctx = ClassifierContext {
            elf_header_range: Some(0x10000..0x10040),
            ..ClassifierContext::default()
        };
        let class = classify(&div(0x40, 1), r.addr, &ctx);
        assert_eq!(class, DivergenceClass::Unclassified);
    }

    #[test]
    fn run_straddling_elf_header_boundary_is_unclassified() {
        let r = region("code", 0x10000, 0x80);
        let ctx = ClassifierContext {
            elf_header_range: Some(0x10000..0x10040),
            ..ClassifierContext::default()
        };
        let class = classify(&div(0x38, 0x10), r.addr, &ctx);
        assert_eq!(class, DivergenceClass::Unclassified);
    }

    #[test]
    fn divergence_exactly_filling_range_classifies() {
        let r = region("code", 0x10000, 0x40);
        let ctx = ClassifierContext {
            elf_header_range: Some(0x10000..0x10040),
            ..ClassifierContext::default()
        };
        assert_eq!(
            classify(&div(0x00, 0x40), r.addr, &ctx),
            DivergenceClass::ElfHeader
        );
    }

    #[test]
    fn divergence_at_range_start_classifies() {
        let r = region("code", 0x10000, 0x40);
        let ctx = ClassifierContext {
            elf_header_range: Some(0x10000..0x10040),
            ..ClassifierContext::default()
        };
        assert_eq!(
            classify(&div(0x00, 1), r.addr, &ctx),
            DivergenceClass::ElfHeader
        );
    }

    #[test]
    fn divergence_ending_at_range_end_classifies() {
        let r = region("code", 0x10000, 0x40);
        let ctx = ClassifierContext {
            elf_header_range: Some(0x10000..0x10040),
            ..ClassifierContext::default()
        };
        assert_eq!(
            classify(&div(0x3F, 1), r.addr, &ctx),
            DivergenceClass::ElfHeader
        );
    }

    #[test]
    fn empty_context_returns_unclassified_for_any_divergence() {
        let r = region("code", 0x10000, 0x40);
        let ctx = ClassifierContext::default();
        assert_eq!(
            classify(&div(0x10, 1), r.addr, &ctx),
            DivergenceClass::Unclassified
        );
    }

    #[test]
    fn sys_proc_param_range_classifies_when_populated() {
        let r = region("code", 0x10000, 0x800);
        let ctx = ClassifierContext {
            sys_proc_param_range: Some(0x10700..0x10720),
            ..ClassifierContext::default()
        };
        let class = classify(&div(0x710, 0x10), r.addr, &ctx);
        assert_eq!(class, DivergenceClass::SysProcParam);
    }

    #[test]
    fn hle_opd_range_classifies_when_populated() {
        let r = region("data", 0x820000, 0x10000);
        let opd_range: Range<u64> = 0x824000..0x824400;
        let ctx = ClassifierContext {
            hle_opd_ranges: vec![opd_range],
            ..ClassifierContext::default()
        };
        let class = classify(&div(0x4100, 0x100), r.addr, &ctx);
        assert_eq!(class, DivergenceClass::HleOpdSlot);
    }

    #[test]
    fn hle_opd_ranges_check_every_entry() {
        let r = region("data", 0x820000, 0x10000);
        let ctx = ClassifierContext {
            hle_opd_ranges: vec![0x820000..0x820004, 0x824000..0x824004, 0x828000..0x828004],
            ..ClassifierContext::default()
        };
        assert_eq!(
            classify(&div(0x0, 4), r.addr, &ctx),
            DivergenceClass::HleOpdSlot
        );
        assert_eq!(
            classify(&div(0x4000, 4), r.addr, &ctx),
            DivergenceClass::HleOpdSlot
        );
        assert_eq!(
            classify(&div(0x8000, 4), r.addr, &ctx),
            DivergenceClass::HleOpdSlot
        );
        assert_eq!(
            classify(&div(0x1000, 4), r.addr, &ctx),
            DivergenceClass::Unclassified
        );
    }

    #[test]
    fn from_observation_populates_elf_header_for_code_region() {
        let ctx = ClassifierContext::from_observation(&obs_with(vec![
            region("code", 0x10000, 0x800000),
            region("data", 0x820000, 0x80000),
        ]))
        .expect("well-formed code region must classify");
        assert_eq!(ctx.elf_header_range, Some(0x10000..0x10040));
        assert!(ctx.sys_proc_param_range.is_none());
        assert!(ctx.hle_opd_ranges.is_empty());
    }

    #[test]
    fn from_observation_picks_first_code_region_when_duplicated() {
        let ctx = ClassifierContext::from_observation(&obs_with(vec![
            region("code", 0x10000, 0x800000),
            region("code", 0x20000, 0x800000),
        ]))
        .expect("first matching code region must classify");
        assert_eq!(ctx.elf_header_range, Some(0x10000..0x10040));
    }

    #[test]
    fn from_observation_rejects_observation_without_code_region() {
        let err =
            ClassifierContext::from_observation(&obs_with(vec![region("data", 0x820000, 0x80000)]))
                .expect_err("observation without code region must Err");
        assert_eq!(err, ClassifierContextError::MissingCodeRegion);
    }

    #[test]
    fn from_observation_rejects_short_code_region() {
        let err = ClassifierContext::from_observation(&obs_with(vec![region("code", 0x10000, 16)]))
            .expect_err("code region shorter than ELF_HEADER_SIZE must Err");
        assert_eq!(
            err,
            ClassifierContextError::ShortCodeRegion {
                len: 16,
                needed: ELF_HEADER_SIZE,
            }
        );
    }

    #[test]
    fn existing_fixture_offsets_classify_as_elf_header() {
        // 0x35 is the low byte of ELF64 `e_ehsize` (BE).
        // 0x17 is the low byte of ELF64 `e_version` (BE).
        let r = region("code", 0x10000, 0x800000);
        let ctx = ClassifierContext::from_observation(&obs_with(vec![r.clone()]))
            .expect("well-formed code region must classify");
        assert_eq!(
            classify(&div(0x35, 1), r.addr, &ctx),
            DivergenceClass::ElfHeader
        );
        assert_eq!(
            classify(&div(0x17, 1), r.addr, &ctx),
            DivergenceClass::ElfHeader
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "ClassifierContext ranges overlap")]
    fn overlapping_ranges_panic_disjoint_check_in_debug() {
        let ctx = ClassifierContext {
            elf_header_range: Some(0x10000..0x10040),
            sys_proc_param_range: Some(0x10020..0x10080),
            hle_opd_ranges: Vec::new(),
            sync_primitive_id_ranges: Vec::new(),
        };
        ctx.debug_assert_disjoint();
    }

    #[test]
    fn abutting_ranges_pass_disjoint_check() {
        let ctx = ClassifierContext {
            elf_header_range: Some(0x10000..0x10040),
            sys_proc_param_range: Some(0x10040..0x10080),
            hle_opd_ranges: Vec::new(),
            sync_primitive_id_ranges: Vec::new(),
        };
        ctx.debug_assert_disjoint();
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "is inverted")]
    fn inverted_range_panics_disjoint_check_in_debug() {
        #[allow(
            clippy::reversed_empty_ranges,
            reason = "range inversion is the regression under test"
        )]
        let ctx = ClassifierContext {
            elf_header_range: Some(0x10040..0x10000),
            sys_proc_param_range: None,
            hle_opd_ranges: Vec::new(),
            sync_primitive_id_ranges: Vec::new(),
        };
        ctx.debug_assert_disjoint();
    }
}
