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
#[path = "tests/classify_tests.rs"]
mod tests;
