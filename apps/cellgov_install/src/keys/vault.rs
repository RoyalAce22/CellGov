//! The loaded vault: where it is found, what it holds, and the
//! accessors the decrypt paths call.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fmt;
use std::path::{Path, PathBuf};

use super::loader::Loader;
use super::{
    CryptoMaterial, IgnoreReason, Ignored, KeyVaultError, Lv2Versions, Provenance, SelfClass,
    SelfKey, Slot,
};

/// Environment variable naming an external vault (file or directory).
pub const ENV_KEYS: &str = "CELLGOV_KEYS";

/// File name of the normalized vault `keys import` writes.
pub const INSTALLED_KEYS_FILE: &str = "keys.toml";

/// The directory under a VFS root where imported keys live.
#[must_use]
pub fn installed_keys_dir(vfs_root: &Path) -> PathBuf {
    vfs_root.join(".cellgov").join("keys")
}

/// A labeled-or-not SELF keyset of one class.
#[derive(Debug, Clone)]
pub(super) struct SelfEntry {
    pub(super) key: SelfKey,
    /// Suffix or section name the keyset was found under.
    pub(super) label: String,
    pub(super) at: Provenance,
}

#[derive(Debug, Clone)]
pub(super) struct SelfTable {
    pub(super) labeled: BTreeMap<u16, SelfEntry>,
    pub(super) unlabeled: Vec<SelfEntry>,
}

impl SelfTable {
    const fn new() -> Self {
        Self {
            labeled: BTreeMap::new(),
            unlabeled: Vec::new(),
        }
    }
}

/// The LV2 keysets: labeled by the firmware versions each opens, or
/// unlabeled.
#[derive(Debug, Clone)]
pub(super) struct Lv2Table {
    pub(super) labeled: BTreeMap<Lv2Versions, SelfEntry>,
    pub(super) unlabeled: Vec<SelfEntry>,
}

impl Lv2Table {
    const fn new() -> Self {
        Self {
            labeled: BTreeMap::new(),
            unlabeled: Vec::new(),
        }
    }
}

/// The loaded key material.
#[derive(Clone)]
pub struct KeyVault {
    pub(super) scalars: BTreeMap<Slot, (Vec<u8>, Provenance)>,
    pub(super) scepkg: Vec<SelfEntry>,
    pub(super) app: SelfTable,
    pub(super) npdrm: SelfTable,
    pub(super) lv2: Lv2Table,
    pub(super) material: Vec<(CryptoMaterial, Provenance)>,
    pub(super) ignored: Vec<Ignored>,
    pub(super) sources: Vec<PathBuf>,
}

impl fmt::Debug for KeyVault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "KeyVault({})", self.summary())
    }
}

impl KeyVault {
    /// Every preserved record with no current decrypt-path consumer.
    #[must_use]
    pub fn material(&self) -> impl ExactSizeIterator<Item = &CryptoMaterial> {
        self.material.iter().map(|(material, _)| material)
    }

    /// A vault holding nothing; every accessor refuses by name.
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            scalars: BTreeMap::new(),
            scepkg: Vec::new(),
            app: SelfTable::new(),
            npdrm: SelfTable::new(),
            lv2: Lv2Table::new(),
            material: Vec::new(),
            ignored: Vec::new(),
            sources: Vec::new(),
        }
    }

    /// Load the vault `CELLGOV_KEYS` names, or the one imported under
    /// `vfs/` at the working directory.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::NotConfigured`] when neither exists, plus every
    /// parse refusal of the one that does.
    pub fn load() -> Result<Self, KeyVaultError> {
        Self::load_for_vfs(Path::new(crate::store::DEFAULT_VFS_ROOT))
    }

    /// [`KeyVault::load`] with an explicit VFS root for the installed
    /// vault.
    ///
    /// # Errors
    ///
    /// Same as [`KeyVault::load`].
    pub fn load_for_vfs(vfs_root: &Path) -> Result<Self, KeyVaultError> {
        let location = Self::locate_from(std::env::var_os(ENV_KEYS), vfs_root)?;
        Self::load_from_path(&location)
    }

    /// Where [`KeyVault::load_for_vfs`] would read from, given the
    /// environment override and the VFS root.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::EnvEmpty`] for a set-but-empty override, and
    /// [`KeyVaultError::NotConfigured`] when the override is absent and
    /// no vault was imported under `vfs_root`.
    pub fn locate_from(
        env_keys: Option<OsString>,
        vfs_root: &Path,
    ) -> Result<PathBuf, KeyVaultError> {
        match env_keys {
            Some(v) if v.is_empty() => Err(KeyVaultError::EnvEmpty),
            Some(v) => Ok(PathBuf::from(v)),
            None => {
                let installed = installed_keys_dir(vfs_root).join(INSTALLED_KEYS_FILE);
                if installed.is_file() {
                    Ok(installed)
                } else {
                    Err(KeyVaultError::NotConfigured { installed })
                }
            }
        }
    }

    /// Load a vault from one file or a directory of files.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::Missing`] for an absent path, I/O refusals,
    /// every parse refusal, and [`KeyVaultError::Conflict`] between two
    /// files.
    pub fn load_from_path(path: &Path) -> Result<Self, KeyVaultError> {
        // `Path::is_dir` answers false to every metadata failure, so the
        // error kind decides between absent and unreadable.
        let meta = match std::fs::metadata(path) {
            Ok(meta) => meta,
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                return Err(KeyVaultError::Missing {
                    path: path.to_path_buf(),
                });
            }
            Err(source) => {
                return Err(KeyVaultError::Io {
                    path: path.to_path_buf(),
                    source,
                });
            }
        };
        let mut loader = Loader::new();
        if meta.is_dir() {
            loader.walk_dir(path, 0)?;
        } else {
            loader.ingest_file(path)?;
        }
        loader.finish()
    }

    /// Parse one keyfile's contents as if read from `label`; the file's
    /// format is decided by `label`'s extension as in
    /// [`KeyVault::load_from_path`].
    ///
    /// # Errors
    ///
    /// Every parse refusal of [`KeyVault::load_from_path`].
    pub fn parse(label: &Path, contents: &[u8]) -> Result<Self, KeyVaultError> {
        let mut loader = Loader::new();
        loader.ingest_bytes(label, contents)?;
        loader.finish()
    }

    /// Fold `other` into this vault; disagreements are refused.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::Conflict`] naming both definitions.
    pub fn merge(&mut self, other: KeyVault) -> Result<(), KeyVaultError> {
        for (slot, (bytes, at)) in other.scalars {
            self.set_scalar(slot, bytes, at)?;
        }
        for entry in other.scepkg {
            self.push_scepkg(entry);
        }
        self.material.extend(other.material);
        for (class, table) in [(SelfClass::App, other.app), (SelfClass::Npdrm, other.npdrm)] {
            for (revision, entry) in table.labeled {
                self.insert_labeled(class, revision, entry)?;
            }
            for entry in table.unlabeled {
                self.push_unlabeled(class, entry);
            }
        }
        for (versions, entry) in other.lv2.labeled {
            self.insert_lv2_labeled(versions, entry)?;
        }
        for entry in other.lv2.unlabeled {
            self.push_unlabeled(SelfClass::Lv2, entry);
        }
        self.ignored.extend(other.ignored);
        self.sources.extend(other.sources);
        Ok(())
    }

    pub(super) fn note_source(&mut self, path: &Path) {
        self.sources.push(path.to_path_buf());
    }

    pub(super) fn ignore(&mut self, at: Provenance, reason: IgnoreReason) {
        self.ignored.push(Ignored { at, reason });
    }

    pub(super) fn set_scalar(
        &mut self,
        slot: Slot,
        bytes: Vec<u8>,
        at: Provenance,
    ) -> Result<(), KeyVaultError> {
        debug_assert_eq!(
            bytes.len(),
            slot.byte_len(),
            "callers length-check before set"
        );
        if let Some((existing, first)) = self.scalars.get(&slot) {
            if *existing != bytes {
                return Err(KeyVaultError::Conflict {
                    what: slot.name().to_string(),
                    first: first.clone(),
                    second: at,
                });
            }
            return Ok(());
        }
        self.scalars.insert(slot, (bytes, at));
        Ok(())
    }

    pub(super) fn push_scepkg(&mut self, entry: SelfEntry) {
        if self.scepkg.iter().all(|e| e.key != entry.key) {
            self.scepkg.push(entry);
        }
    }

    /// The table of `class` whose labels are revisions; `None` for the
    /// LV2 class, whose labels are versions.
    pub(super) fn revision_table(&self, class: SelfClass) -> Option<&SelfTable> {
        match class {
            SelfClass::App => Some(&self.app),
            SelfClass::Npdrm => Some(&self.npdrm),
            SelfClass::Lv2 => None,
        }
    }

    /// `(labeled, unlabeled)` for `class`; only the unlabeled list is
    /// mutable.
    fn tables_mut(&mut self, class: SelfClass) -> (Vec<&SelfEntry>, &mut Vec<SelfEntry>) {
        match class {
            SelfClass::App => (self.app.labeled.values().collect(), &mut self.app.unlabeled),
            SelfClass::Npdrm => (
                self.npdrm.labeled.values().collect(),
                &mut self.npdrm.unlabeled,
            ),
            SelfClass::Lv2 => (self.lv2.labeled.values().collect(), &mut self.lv2.unlabeled),
        }
    }

    /// File `entry` under `revision` in an APP or NPDRM table.
    ///
    /// # Panics
    ///
    /// If `class` is [`SelfClass::Lv2`]: no revision labels one, and
    /// [`KeyVault::insert_lv2_labeled`] files it.
    pub(super) fn insert_labeled(
        &mut self,
        class: SelfClass,
        revision: u16,
        entry: SelfEntry,
    ) -> Result<(), KeyVaultError> {
        let table = match class {
            SelfClass::App => &mut self.app,
            SelfClass::Npdrm => &mut self.npdrm,
            SelfClass::Lv2 => panic!("an LV2 keyset is labeled by version, not revision"),
        };
        if let Some(existing) = table.labeled.get(&revision) {
            if existing.key != entry.key {
                return Err(KeyVaultError::Conflict {
                    what: format!("{class} revision 0x{revision:04x}"),
                    first: existing.at.clone(),
                    second: entry.at,
                });
            }
            return Ok(());
        }
        // Dropping the same keyset from the candidates here, and
        // `push_unlabeled` skipping a key that is already labeled, keep
        // the vault independent of walk order.
        table.unlabeled.retain(|e| e.key != entry.key);
        table.labeled.insert(revision, entry);
        Ok(())
    }

    /// File `entry` under the firmware versions it opens.
    pub(super) fn insert_lv2_labeled(
        &mut self,
        versions: Lv2Versions,
        entry: SelfEntry,
    ) -> Result<(), KeyVaultError> {
        if let Some(existing) = self.lv2.labeled.get(&versions) {
            if existing.key != entry.key {
                return Err(KeyVaultError::Conflict {
                    what: format!("lv2 versions {versions}"),
                    first: existing.at.clone(),
                    second: entry.at,
                });
            }
            return Ok(());
        }
        self.lv2.unlabeled.retain(|e| e.key != entry.key);
        self.lv2.labeled.insert(versions, entry);
        Ok(())
    }

    pub(super) fn push_unlabeled(&mut self, class: SelfClass, entry: SelfEntry) {
        let (labeled, unlabeled) = self.tables_mut(class);
        let known = labeled.iter().any(|e| e.key == entry.key)
            || unlabeled.iter().any(|e| e.key == entry.key);
        if !known {
            unlabeled.push(entry);
        }
    }

    pub(super) fn scalar(&self, slot: Slot) -> Result<&[u8], KeyVaultError> {
        self.scalars
            .get(&slot)
            .map(|(bytes, _)| bytes.as_slice())
            .ok_or(KeyVaultError::MissingSlot { slot })
    }

    fn scalar16(&self, slot: Slot) -> Result<&[u8; 16], KeyVaultError> {
        self.scalar(slot)?
            .try_into()
            .map_err(|_| KeyVaultError::MissingSlot { slot })
    }

    /// HMAC-SHA1 key over PUP payloads.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::MissingSlot`] when the vault has none.
    pub fn pup_hmac(&self) -> Result<&[u8; 0x40], KeyVaultError> {
        self.scalar(Slot::PupHmac)?
            .try_into()
            .map_err(|_| KeyVaultError::MissingSlot {
                slot: Slot::PupHmac,
            })
    }

    /// AES-128 key of the retail PKG CTR keystream.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::MissingSlot`] when the vault has none.
    pub fn pkg_aes(&self) -> Result<&[u8; 16], KeyVaultError> {
        self.scalar16(Slot::PkgAes)
    }

    /// AES-128 key that turns a klicensee into the NPDRM layer key.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::MissingSlot`] when the vault has none.
    pub fn np_klic_key(&self) -> Result<&[u8; 16], KeyVaultError> {
        self.scalar16(Slot::NpKlicKey)
    }

    /// Klicensee of free-license NPDRM titles with no RAP.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::MissingSlot`] when the vault has none.
    pub fn np_klic_free(&self) -> Result<&[u8; 16], KeyVaultError> {
        self.scalar16(Slot::NpKlicFree)
    }

    /// AES-128 key of the RAP derivation's ECB stage.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::MissingSlot`] when the vault has none.
    pub fn rap_key(&self) -> Result<&[u8; 16], KeyVaultError> {
        self.scalar16(Slot::RapKey)
    }

    /// Byte permutation of the RAP derivation rounds.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::MissingSlot`] when the vault has none.
    pub fn rap_pbox(&self) -> Result<&[u8; 16], KeyVaultError> {
        self.scalar16(Slot::RapPbox)
    }

    /// First per-round table of the RAP derivation.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::MissingSlot`] when the vault has none.
    pub fn rap_e1(&self) -> Result<&[u8; 16], KeyVaultError> {
        self.scalar16(Slot::RapE1)
    }

    /// Second per-round table of the RAP derivation.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::MissingSlot`] when the vault has none.
    pub fn rap_e2(&self) -> Result<&[u8; 16], KeyVaultError> {
        self.scalar16(Slot::RapE2)
    }

    /// SCE package (firmware PKG envelope) keysets, in load order; a
    /// decrypt tries each until the envelope padding checks.
    ///
    /// # Errors
    ///
    /// [`KeyVaultError::MissingScepkg`] when the vault holds none.
    pub fn scepkg_keys(&self) -> Result<impl Iterator<Item = &SelfKey> + '_, KeyVaultError> {
        if self.scepkg.is_empty() {
            return Err(KeyVaultError::MissingScepkg);
        }
        Ok(self.scepkg.iter().map(|e| &e.key))
    }

    /// The keyset labeled `revision` in `class`'s table; `None` for
    /// [`SelfClass::Lv2`], whose labels are versions.
    #[must_use]
    pub fn self_key(&self, class: SelfClass, revision: u16) -> Option<&SelfKey> {
        self.revision_table(class)?
            .labeled
            .get(&revision)
            .map(|e| &e.key)
    }

    /// Every keyset that may open an APP or NPDRM SELF of `revision`:
    /// the labeled one first, then each unlabeled candidate in load
    /// order. The iterator is empty for [`SelfClass::Lv2`]; see
    /// [`KeyVault::lv2_key_candidates`].
    pub fn self_key_candidates(
        &self,
        class: SelfClass,
        revision: u16,
    ) -> impl Iterator<Item = &SelfKey> + '_ {
        self.revision_table(class)
            .into_iter()
            .flat_map(move |table| {
                table
                    .labeled
                    .get(&revision)
                    .into_iter()
                    .chain(table.unlabeled.iter())
            })
            .map(|e| &e.key)
    }

    /// Every LV2 keyset that may open a kernel of `version`, in order:
    ///
    /// 1. those labeled for a range holding `version`;
    /// 2. those labeled for other ranges;
    /// 3. each unlabeled candidate, in load order.
    ///
    /// The decrypt still tries a keyset labeled for other versions: the
    /// label is the operator's, and the envelope's padding is the check.
    pub fn lv2_key_candidates(&self, version: u64) -> impl Iterator<Item = &SelfKey> + '_ {
        let (holding, other): (Vec<_>, Vec<_>) = self
            .lv2
            .labeled
            .iter()
            .partition(|(versions, _)| versions.contains(version));
        holding
            .into_iter()
            .chain(other)
            .map(|(_, e)| e)
            .chain(self.lv2.unlabeled.iter())
            .map(|e| &e.key)
    }

    /// The revisions that label keysets in `class`'s table; none for
    /// [`SelfClass::Lv2`].
    pub fn labeled_revisions(&self, class: SelfClass) -> impl Iterator<Item = u16> + '_ {
        self.revision_table(class)
            .into_iter()
            .flat_map(|table| table.labeled.keys().copied())
    }

    /// The version ranges that label keysets in the LV2 table.
    pub fn lv2_versions(&self) -> impl Iterator<Item = Lv2Versions> + '_ {
        self.lv2.labeled.keys().copied()
    }

    /// The labels of `class`'s keysets as the inventory prints them:
    /// `0x000a` revisions, or `3.60-3.61` version ranges.
    #[must_use]
    pub fn labels(&self, class: SelfClass) -> Vec<String> {
        match class {
            SelfClass::App | SelfClass::Npdrm => self
                .labeled_revisions(class)
                .map(|r| format!("0x{r:04x}"))
                .collect(),
            SelfClass::Lv2 => self.lv2_versions().map(|v| v.to_string()).collect(),
        }
    }

    /// Number of unlabeled candidates in `class`'s table.
    #[must_use]
    pub fn unlabeled_count(&self, class: SelfClass) -> usize {
        match class {
            SelfClass::App => self.app.unlabeled.len(),
            SelfClass::Npdrm => self.npdrm.unlabeled.len(),
            SelfClass::Lv2 => self.lv2.unlabeled.len(),
        }
    }

    /// Number of keysets `class` holds, labeled or not.
    #[must_use]
    pub fn keyset_count(&self, class: SelfClass) -> usize {
        let labeled = match class {
            SelfClass::App => self.app.labeled.len(),
            SelfClass::Npdrm => self.npdrm.labeled.len(),
            SelfClass::Lv2 => self.lv2.labeled.len(),
        };
        labeled.saturating_add(self.unlabeled_count(class))
    }

    /// Everything the loader read and set aside, with the reason.
    #[must_use]
    pub fn ignored(&self) -> &[Ignored] {
        &self.ignored
    }

    /// Files the vault was read from.
    #[must_use]
    pub fn sources(&self) -> &[PathBuf] {
        &self.sources
    }

    /// Where a scalar slot's value came from.
    #[must_use]
    pub fn slot_provenance(&self, slot: Slot) -> Option<&Provenance> {
        self.scalars.get(&slot).map(|(_, at)| at)
    }

    /// What every decrypt path needs and this vault lacks, by name.
    #[must_use]
    pub fn missing_for_decrypt(&self) -> Vec<String> {
        let mut missing: Vec<String> = Slot::ALL
            .iter()
            .filter(|s| !self.scalars.contains_key(s))
            .map(|s| s.name().to_string())
            .collect();
        if self.scepkg.is_empty() {
            missing.push("scepkg".to_string());
        }
        for class in SelfClass::ALL {
            if self.keyset_count(class) == 0 {
                missing.push(format!("{class} (no keyset)"));
            }
        }
        missing
    }

    /// Whether the vault holds any value a decrypt path could use: a
    /// scalar slot, an SCE package key, or a SELF keyset of any class.
    /// Preserved material alone does not count.
    #[must_use]
    pub fn holds_any_key(&self) -> bool {
        !self.scalars.is_empty()
            || !self.scepkg.is_empty()
            || SelfClass::ALL.iter().any(|c| self.keyset_count(*c) > 0)
    }

    /// One-line count summary.
    #[must_use]
    pub fn summary(&self) -> String {
        format!(
            "{} of {} scalar slots, {} scepkg, app {}+{}, npdrm {}+{}, lv2 {}+{}, {} preserved, {} ignored",
            self.scalars.len(),
            Slot::ALL.len(),
            self.scepkg.len(),
            self.app.labeled.len(),
            self.app.unlabeled.len(),
            self.npdrm.labeled.len(),
            self.npdrm.unlabeled.len(),
            self.lv2.labeled.len(),
            self.lv2.unlabeled.len(),
            self.material.len(),
            self.ignored.len(),
        )
    }
}
