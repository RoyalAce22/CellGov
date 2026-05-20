//! Divergence classifier: maps each [`ByteDivergence`] to a known
//! non-semantic class or `Unclassified`. Each class names a
//! structural mechanism; bytes that do not fit a named mechanism
//! stay `Unclassified`.

use std::ops::Range;

use serde::{Deserialize, Serialize};

use crate::observation::{Observation, CODE_REGION_NAME};
use crate::observation_compare::ByteDivergence;

pub use cellgov_ps3_abi::elf::ELF_HEADER_SIZE;

/// Classified shape of a single byte-divergence run.
///
/// `Ord` is derived to give [`per_class_bytes`](crate::CrossRunnerSummary)
/// a deterministic BTreeMap key order; the discriminant ordering is
/// load-bearing for that contract (see `summary.rs`'s
/// `per_class_bytes_iterates_in_discriminant_order` test).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DivergenceClass {
    /// Bytes inside the loaded ELF header. Non-semantic; the running
    /// program never reads them.
    ElfHeader,
    /// Bytes inside the `sys_process_param_t` struct's load location.
    /// Non-semantic; both runners read the parsed fields via internal
    /// state, not by re-reading the loaded bytes.
    SysProcParam,
    /// Bytes inside an HLE OPD trampoline slot. Non-semantic;
    /// per-slot pointer indices differ across runners but the
    /// resolved entry points are equivalent.
    HleOpdSlot,
    /// No populated context range contained this divergence run.
    /// Counted in the byte-parity Pending bucket and enumerated in
    /// `cross_runner_summary.json`'s `unclassified_runs`.
    Unclassified,
}

impl DivergenceClass {
    pub fn is_non_semantic(&self) -> bool {
        match self {
            Self::ElfHeader | Self::SysProcParam | Self::HleOpdSlot => true,
            Self::Unclassified => false,
        }
    }
}

/// Fixed identifier per variant; pinned by
/// `divergence_class_display_strings_are_stable`. Renderers that
/// produce on-disk fixtures (`compare_report.txt`, hand-authored
/// `NOTES.md`) read this rather than `{:?}` so a future rename
/// cannot silently churn the text.
impl std::fmt::Display for DivergenceClass {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::ElfHeader => "ElfHeader",
            Self::SysProcParam => "SysProcParam",
            Self::HleOpdSlot => "HleOpdSlot",
            Self::Unclassified => "Unclassified",
        })
    }
}

/// Pre-computed guest-address ranges the classifier checks for
/// containment. Each range is `None` (or empty Vec) until its
/// corresponding populator slice lands.
///
/// `hle_opd_ranges` may contain multiple structurally distinct
/// range kinds, all classifying as `HleOpdSlot`:
///   1. Primary function-stub table -- one contiguous range covering
///      the title's main OPD slot table (from the SCE PRX_PARAM
///      `lib_stub_start..lib_stub_end` import area).
///   2. Variable-stub slots -- zero or more 4-byte ranges for the
///      scattered variable-stub slots that don't share the primary
///      table's contiguity.
///   3. Secondary OPD tables -- zero or more contiguous ranges for
///      the FNID-walker-patched sibling tables identified by the
///      `0x04020100` / `0x04020200` header signature (typically two
///      per title, adjacent and merged into one Range; see SSHD
///      `0x829b10`/`0x829b78` and WipEout `0x925008`/`0x925070`).
///
/// All entries must be pairwise non-overlapping. Verified by
/// [`debug_assert_disjoint`](Self::debug_assert_disjoint); if a
/// future title's layout violates this, the assert fires before
/// the classifier sees the corrupted ranges.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassifierContext {
    pub elf_header_range: Option<Range<u64>>,
    pub sys_proc_param_range: Option<Range<u64>>,
    pub hle_opd_ranges: Vec<Range<u64>>,
}

impl ClassifierContext {
    /// Build a context with only `elf_header_range` populated from
    /// the observation's `"code"` region. Real boots build the
    /// fuller context from EBOOT bytes; this path is for synthetic
    /// fixtures.
    ///
    /// # Panics
    ///
    /// In debug, if the observation has no `"code"` region or that
    /// region carries fewer than [`ELF_HEADER_SIZE`] bytes.
    pub fn from_observation(obs: &Observation) -> Self {
        let code = obs
            .memory_regions
            .iter()
            .find(|r| r.name == CODE_REGION_NAME);
        debug_assert!(
            code.is_some(),
            "from_observation called on observation lacking the {CODE_REGION_NAME:?} region"
        );
        let elf_header_range = code.map(|r| {
            debug_assert!(
                r.data.len() >= ELF_HEADER_SIZE,
                "{CODE_REGION_NAME:?} region carries {} bytes (< ELF_HEADER_SIZE = {})",
                r.data.len(),
                ELF_HEADER_SIZE
            );
            let end = r
                .addr
                .checked_add(ELF_HEADER_SIZE as u64)
                .expect("code region addr + ELF_HEADER_SIZE overflows u64");
            r.addr..end
        });
        let ctx = Self {
            elf_header_range,
            sys_proc_param_range: None,
            hle_opd_ranges: Vec::new(),
        };
        // Empty pairwise loop today (only one range populated); the
        // check becomes load-bearing if this constructor grows.
        ctx.debug_assert_disjoint();
        ctx
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
/// of `ctx`'s named ranges. Partial overlap returns `Unclassified`:
/// a cross-class divergence needs human attention rather than
/// vote-by-first-byte.
///
/// `region_addr` is the guest address of the region's first byte
/// (i.e. the `addr` field of the [`NamedMemoryRegion`] the
/// divergence belongs to); the classifier needs no other field.
///
/// Check order (`ElfHeader`, `SysProcParam`, `HleOpdSlot`) is
/// irrelevant for [disjoint](ClassifierContext::debug_assert_disjoint)
/// contexts.
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
    DivergenceClass::Unclassified
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::observation::{NamedMemoryRegion, ObservationMetadata, ObservedOutcome};

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
        assert_eq!(format!("{}", DivergenceClass::Unclassified), "Unclassified");
    }

    #[test]
    fn divergence_class_is_non_semantic_for_three_known_classes() {
        assert!(DivergenceClass::ElfHeader.is_non_semantic());
        assert!(DivergenceClass::SysProcParam.is_non_semantic());
        assert!(DivergenceClass::HleOpdSlot.is_non_semantic());
        assert!(!DivergenceClass::Unclassified.is_non_semantic());
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
        ]));
        assert_eq!(ctx.elf_header_range, Some(0x10000..0x10040));
        assert!(ctx.sys_proc_param_range.is_none());
        assert!(ctx.hle_opd_ranges.is_empty());
    }

    #[test]
    fn from_observation_picks_first_code_region_when_duplicated() {
        let ctx = ClassifierContext::from_observation(&obs_with(vec![
            region("code", 0x10000, 0x800000),
            region("code", 0x20000, 0x800000),
        ]));
        assert_eq!(ctx.elf_header_range, Some(0x10000..0x10040));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "lacking the \"code\" region")]
    fn from_observation_panics_without_code_region_in_debug() {
        ClassifierContext::from_observation(&obs_with(vec![region("data", 0x820000, 0x80000)]));
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "< ELF_HEADER_SIZE")]
    fn from_observation_panics_on_short_code_region_in_debug() {
        ClassifierContext::from_observation(&obs_with(vec![region("code", 0x10000, 16)]));
    }

    #[test]
    fn existing_fixture_offsets_classify_as_elf_header() {
        // 0x35 is the low byte of ELF64 `e_ehsize` (BE).
        // 0x17 is the low byte of ELF64 `e_version` (BE).
        let r = region("code", 0x10000, 0x800000);
        let ctx = ClassifierContext::from_observation(&obs_with(vec![r.clone()]));
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
        };
        ctx.debug_assert_disjoint();
    }

    #[test]
    fn abutting_ranges_pass_disjoint_check() {
        let ctx = ClassifierContext {
            elf_header_range: Some(0x10000..0x10040),
            sys_proc_param_range: Some(0x10040..0x10080),
            hle_opd_ranges: Vec::new(),
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
        };
        ctx.debug_assert_disjoint();
    }
}
