//! Loaded-PRX registry consulted by `_sys_prx_load_module`,
//! `_sys_prx_get_module_list`, and `_sys_prx_get_module_info`.
//! Populated at firmware-set boot; lookups resolve by path-stem
//! or by kernel id.

use std::collections::BTreeMap;

/// First kernel id handed out. The range
/// `0x4000_0001..0x4002_0000` is reserved for `Lv2Host::alloc_id`
/// and the allocator chains it shares; this struct owns
/// `FIRST_KERNEL_ID` and above.
pub const FIRST_KERNEL_ID: u32 = 0x4002_0000;

/// One entry per loaded PRX.
#[derive(Debug, Clone)]
pub struct LoadedPrxEntry {
    kernel_id: u32,
    stem: String,
    name: String,
    base: u32,
    data_end: u32,
    toc: u32,
    start_opd: Option<u32>,
    stop_opd: Option<u32>,
}

impl LoadedPrxEntry {
    /// Kernel-handed id; opaque to the guest, monotonic per host instance.
    pub fn kernel_id(&self) -> u32 {
        self.kernel_id
    }

    /// File-system stem (e.g. `"libaudio"`). Used by path lookup.
    pub fn stem(&self) -> &str {
        &self.stem
    }

    /// Module-info name (e.g. `"cellAudio_Library"`).
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Runtime base address of segment 0.
    pub fn base(&self) -> u32 {
        self.base
    }

    /// Exclusive end of the data segment.
    pub fn data_end(&self) -> u32 {
        self.data_end
    }

    /// Runtime TOC address.
    pub fn toc(&self) -> u32 {
        self.toc
    }

    /// Runtime address of the `module_start` OPD, if exported.
    pub fn start_opd(&self) -> Option<u32> {
        self.start_opd
    }

    /// Runtime address of the `module_stop` OPD, if exported.
    pub fn stop_opd(&self) -> Option<u32> {
        self.stop_opd
    }
}

/// Table of loaded PRXs keyed by both kernel id and stem.
///
/// Invariant: every key in `stem_to_id` resolves to a present
/// `entries` row. [`register`](Self::register) is the only
/// mutating surface; [`lookup_by_path`](Self::lookup_by_path)
/// debug-asserts the invariant.
#[derive(Debug, Clone)]
pub struct LoadedPrxRegistry {
    entries: BTreeMap<u32, LoadedPrxEntry>,
    stem_to_id: BTreeMap<String, u32>,
    next_id: u32,
}

impl LoadedPrxRegistry {
    /// Construct an empty registry. First-issued id is
    /// [`FIRST_KERNEL_ID`].
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            stem_to_id: BTreeMap::new(),
            next_id: FIRST_KERNEL_ID,
        }
    }

    /// Add a loaded PRX. Returns the freshly minted kernel id.
    ///
    /// `stem` must already match [`extract_stem`] output: no
    /// directory prefix, no `.prx`/`.sprx` suffix, non-empty.
    /// Address invariants: `base <= data_end`, and `start_opd` /
    /// `stop_opd` (when `Some`) fall inside `[base, data_end)`.
    ///
    /// # Panics
    ///
    /// Panics if `stem` is already registered. In debug builds also
    /// panics on the invariant violations above and on id-space
    /// exhaustion.
    #[allow(clippy::too_many_arguments)]
    pub fn register(
        &mut self,
        stem: String,
        name: String,
        base: u32,
        data_end: u32,
        toc: u32,
        start_opd: Option<u32>,
        stop_opd: Option<u32>,
    ) -> u32 {
        debug_assert!(
            !stem.is_empty(),
            "LoadedPrxRegistry::register: stem must be non-empty"
        );
        debug_assert_eq!(
            stem,
            extract_stem(&stem),
            "LoadedPrxRegistry::register: stem {stem:?} is not already \
             normalised (must match extract_stem(stem))"
        );
        debug_assert!(
            data_end >= base,
            "LoadedPrxRegistry::register: data_end {data_end:#x} < base {base:#x}"
        );
        if let Some(opd) = start_opd {
            debug_assert!(
                opd >= base && opd < data_end,
                "LoadedPrxRegistry::register: start_opd {opd:#x} outside \
                 [base={base:#x}, data_end={data_end:#x})"
            );
        }
        if let Some(opd) = stop_opd {
            debug_assert!(
                opd >= base && opd < data_end,
                "LoadedPrxRegistry::register: stop_opd {opd:#x} outside \
                 [base={base:#x}, data_end={data_end:#x})"
            );
        }
        assert!(
            !self.stem_to_id.contains_key(&stem),
            "LoadedPrxRegistry::register: stem {stem:?} already registered"
        );

        let kernel_id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .expect("LoadedPrxRegistry: next kernel id cannot be allocated");
        self.entries.insert(
            kernel_id,
            LoadedPrxEntry {
                kernel_id,
                stem: stem.clone(),
                name,
                base,
                data_end,
                toc,
                start_opd,
                stop_opd,
            },
        );
        self.stem_to_id.insert(stem, kernel_id);
        kernel_id
    }

    /// Look up an entry by guest-supplied path; matched on the
    /// extracted stem. Empty stem returns `None`.
    pub fn lookup_by_path(&self, path: &str) -> Option<&LoadedPrxEntry> {
        let stem = extract_stem(path);
        if stem.is_empty() {
            return None;
        }
        let id = self.stem_to_id.get(stem.as_str())?;
        let entry = self.entries.get(id);
        debug_assert!(
            entry.is_some(),
            "LoadedPrxRegistry: stem_to_id and entries out of sync (id={id:#x})"
        );
        entry
    }

    /// Look up an entry by kernel id.
    pub fn lookup_by_id(&self, id: u32) -> Option<&LoadedPrxEntry> {
        self.entries.get(&id)
    }

    /// Number of registered PRXs.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the registry has any entries.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterate kernel ids in monotonic (BTreeMap key) order.
    pub fn ids(&self) -> impl Iterator<Item = u32> + '_ {
        self.entries.keys().copied()
    }
}

impl Default for LoadedPrxRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Strip directory prefix and a single trailing `.sprx` or `.prx`
/// extension. Forward slashes and backslashes both terminate the
/// basename so a Windows host path is treated the same as a PS3 VFS
/// path. Matching is case-sensitive. Inputs that normalise to the
/// empty string (`""`, `".sprx"`, `"foo/"`, ...) are returned as
/// `""`; [`LoadedPrxRegistry::lookup_by_path`] rejects those.
pub fn extract_stem(path: &str) -> String {
    let basename = path.rsplit(['/', '\\']).next().unwrap_or(path);
    if let Some(stem) = basename.strip_suffix(".sprx") {
        return stem.to_string();
    }
    if let Some(stem) = basename.strip_suffix(".prx") {
        return stem.to_string();
    }
    basename.to_string()
}

#[cfg(test)]
#[path = "tests/prx_registry_tests.rs"]
mod tests;
