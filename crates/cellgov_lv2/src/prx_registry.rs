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
mod tests {
    use super::*;

    // -- extract_stem --

    #[test]
    fn extract_stem_strips_directory_and_sprx() {
        assert_eq!(
            extract_stem("/dev_flash/sys/external/libaudio.sprx"),
            "libaudio"
        );
        assert_eq!(extract_stem("external/libaudio.sprx"), "libaudio");
        assert_eq!(extract_stem("libaudio.sprx"), "libaudio");
        assert_eq!(extract_stem("libaudio.prx"), "libaudio");
        assert_eq!(extract_stem("libaudio"), "libaudio");
    }

    #[test]
    fn extract_stem_handles_windows_separators() {
        assert_eq!(extract_stem("D:\\foo\\bar\\libaudio.sprx"), "libaudio");
    }

    #[test]
    fn extract_stem_empty_inputs_return_empty() {
        assert_eq!(extract_stem(""), "");
        assert_eq!(extract_stem(".sprx"), "");
        assert_eq!(extract_stem(".prx"), "");
        assert_eq!(extract_stem("/"), "");
        assert_eq!(extract_stem("foo/"), "");
    }

    #[test]
    fn extract_stem_unrecognised_extension_passes_through() {
        assert_eq!(extract_stem("libfoo.bar"), "libfoo.bar");
    }

    #[test]
    fn extract_stem_is_case_sensitive() {
        assert_eq!(extract_stem("libaudio.SPRX"), "libaudio.SPRX");
        assert_eq!(extract_stem("libaudio.Prx"), "libaudio.Prx");
    }

    // -- register / lookup happy path --

    fn register_libaudio(reg: &mut LoadedPrxRegistry) -> u32 {
        reg.register(
            "libaudio".to_string(),
            "cellAudio_Library".to_string(),
            0x0147_0000,
            0x0148_0000,
            0x0147_da30,
            Some(0x0147_1000),
            Some(0x0147_2000),
        )
    }

    #[test]
    fn register_then_lookup_by_path_finds_entry() {
        let mut reg = LoadedPrxRegistry::new();
        let id = register_libaudio(&mut reg);
        let entry = reg
            .lookup_by_path("external/libaudio.sprx")
            .expect("libaudio resolves");
        assert_eq!(entry.kernel_id(), id);
        assert_eq!(entry.name(), "cellAudio_Library");
        assert_eq!(entry.base(), 0x0147_0000);
        assert_eq!(
            reg.lookup_by_path("/dev_flash/sys/external/libaudio.sprx")
                .unwrap()
                .kernel_id(),
            id
        );
    }

    #[test]
    fn lookup_by_unknown_path_returns_none() {
        let mut reg = LoadedPrxRegistry::new();
        register_libaudio(&mut reg);
        assert!(reg.lookup_by_path("external/libfoo.sprx").is_none());
    }

    #[test]
    fn lookup_by_empty_path_returns_none() {
        let mut reg = LoadedPrxRegistry::new();
        register_libaudio(&mut reg);
        assert!(reg.lookup_by_path("").is_none());
        assert!(reg.lookup_by_path(".sprx").is_none());
        assert!(reg.lookup_by_path("foo/").is_none());
    }

    #[test]
    fn lookup_by_id_returns_registered_entry() {
        let mut reg = LoadedPrxRegistry::new();
        let id = register_libaudio(&mut reg);
        assert!(reg.lookup_by_id(id).is_some());
        assert!(reg.lookup_by_id(id + 1).is_none());
    }

    #[test]
    fn lookup_by_id_and_path_return_same_entry() {
        let mut reg = LoadedPrxRegistry::new();
        let id = register_libaudio(&mut reg);
        let by_id = reg.lookup_by_id(id).unwrap();
        let by_path = reg.lookup_by_path("libaudio.sprx").unwrap();
        assert!(std::ptr::eq(by_id, by_path));
    }

    #[test]
    fn round_trip_preserves_every_field() {
        let mut reg = LoadedPrxRegistry::new();
        let id = reg.register(
            "libsentinel".to_string(),
            "X_Y_Z".to_string(),
            0x1111_0000,
            0x1112_0000,
            0x1111_d000,
            Some(0x1111_1000),
            Some(0x1111_2000),
        );
        let entry = reg.lookup_by_id(id).unwrap();
        assert_eq!(entry.kernel_id(), id);
        assert_eq!(entry.stem(), "libsentinel");
        assert_eq!(entry.name(), "X_Y_Z");
        assert_eq!(entry.base(), 0x1111_0000);
        assert_eq!(entry.data_end(), 0x1112_0000);
        assert_eq!(entry.toc(), 0x1111_d000);
        assert_eq!(entry.start_opd(), Some(0x1111_1000));
        assert_eq!(entry.stop_opd(), Some(0x1111_2000));
    }

    // -- ids / counters --

    #[test]
    fn first_kernel_id_is_named_constant() {
        let mut reg = LoadedPrxRegistry::new();
        let id = register_libaudio(&mut reg);
        assert_eq!(id, FIRST_KERNEL_ID);
    }

    #[test]
    fn consecutive_ids_increment_by_one() {
        let mut reg = LoadedPrxRegistry::new();
        let a = reg.register("liba".into(), "A".into(), 0, 0, 0, None, None);
        let b = reg.register("libb".into(), "B".into(), 0, 0, 0, None, None);
        let c = reg.register("libc".into(), "C".into(), 0, 0, 0, None, None);
        assert_eq!(b, a + 1);
        assert_eq!(c, a + 2);
    }

    #[test]
    fn is_empty_tracks_registration() {
        let mut reg = LoadedPrxRegistry::new();
        assert!(reg.is_empty());
        register_libaudio(&mut reg);
        assert!(!reg.is_empty());
    }

    #[test]
    fn ids_on_empty_registry_yields_nothing() {
        let reg = LoadedPrxRegistry::new();
        assert_eq!(reg.ids().count(), 0);
    }

    #[test]
    fn ids_iterates_in_monotonic_order() {
        let mut reg = LoadedPrxRegistry::new();
        reg.register("liba".into(), "A".into(), 0, 0, 0, None, None);
        reg.register("libb".into(), "B".into(), 0, 0, 0, None, None);
        reg.register("libc".into(), "C".into(), 0, 0, 0, None, None);
        let ids: Vec<u32> = reg.ids().collect();
        assert_eq!(ids.len(), 3);
        assert!(ids[0] < ids[1]);
        assert!(ids[1] < ids[2]);
    }

    // -- duplicate registration --

    #[test]
    #[should_panic(expected = "already registered")]
    fn register_same_stem_twice_panics() {
        let mut reg = LoadedPrxRegistry::new();
        reg.register("libaudio".into(), "A".into(), 0, 0, 0, None, None);
        reg.register("libaudio".into(), "B".into(), 0, 0, 0, None, None);
    }

    // -- precondition guards (debug-only) --

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "non-empty")]
    fn register_empty_stem_panics_debug() {
        let mut reg = LoadedPrxRegistry::new();
        reg.register(String::new(), "A".into(), 0, 0, 0, None, None);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "not already normalised")]
    fn register_unstripped_extension_panics_debug() {
        let mut reg = LoadedPrxRegistry::new();
        reg.register("libaudio.sprx".into(), "A".into(), 0, 0, 0, None, None);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "not already normalised")]
    fn register_dir_prefix_panics_debug() {
        let mut reg = LoadedPrxRegistry::new();
        reg.register("foo/bar".into(), "A".into(), 0, 0, 0, None, None);
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "data_end")]
    fn register_data_end_below_base_panics_debug() {
        let mut reg = LoadedPrxRegistry::new();
        reg.register(
            "libaudio".into(),
            "A".into(),
            0x1000_0000,
            0x0FFF_F000,
            0x1000_8000,
            None,
            None,
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "start_opd")]
    fn register_start_opd_outside_segment_panics_debug() {
        let mut reg = LoadedPrxRegistry::new();
        reg.register(
            "libaudio".into(),
            "A".into(),
            0x1000_0000,
            0x1010_0000,
            0x1000_8000,
            Some(0x2000_0000),
            None,
        );
    }

    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "stop_opd")]
    fn register_stop_opd_outside_segment_panics_debug() {
        let mut reg = LoadedPrxRegistry::new();
        reg.register(
            "libaudio".into(),
            "A".into(),
            0x1000_0000,
            0x1010_0000,
            0x1000_8000,
            None,
            Some(0x2000_0000),
        );
    }
}
