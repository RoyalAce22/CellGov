//! Reading keyfiles: the directory walk, per-file classification, the
//! text grammar (scetool blocks, named lines, table rows), and the
//! pairing of keyset halves that arrive in separate files.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::hex::{decode_hex, is_hex_token, value_bytes};
use super::names::{classify_name, normalized, parse_revision, words, Kind, NameClass, Part};
use super::vault::SelfEntry;
use super::{IgnoreReason, KeyVault, KeyVaultError, Lv2Versions, Provenance, SelfClass, SelfKey};

#[cfg(test)]
#[path = "tests/loader_tests.rs"]
mod tests;

/// Largest file the directory walk reads; bigger ones are listed as
/// ignored unread.
const MAX_KEY_FILE_BYTES: u64 = 1024 * 1024;

/// Directory depth the vault walk descends below the root.
const MAX_WALK_DEPTH: usize = 4;

/// One `[section]` of a text keyfile while its properties accumulate.
struct Block {
    name: String,
    line: usize,
    props: BTreeMap<String, (String, usize)>,
}

/// A keyset half waiting for its other half.
pub(super) struct PendingHalf {
    pub(super) bytes: Vec<u8>,
    pub(super) what: String,
    pub(super) at: Provenance,
}

/// Accumulates one [`KeyVault`] across files, pairing keyset halves at
/// the end.
pub(super) struct Loader {
    pub(super) vault: KeyVault,
    pending: BTreeMap<(Kind, String), BTreeMap<Part, PendingHalf>>,
}

impl Loader {
    pub(super) fn new() -> Self {
        Self {
            vault: KeyVault::empty(),
            pending: BTreeMap::new(),
        }
    }

    pub(super) fn finish(mut self) -> Result<KeyVault, KeyVaultError> {
        let pending = std::mem::take(&mut self.pending);
        for ((kind, label), mut halves) in pending {
            let (erk, riv) = match (halves.remove(&Part::Erk), halves.remove(&Part::Riv)) {
                (Some(erk), Some(riv)) => (erk, riv),
                (Some(only), None) => {
                    self.unpaired(only, Part::Erk);
                    continue;
                }
                (None, Some(only)) => {
                    self.unpaired(only, Part::Riv);
                    continue;
                }
                (None, None) => continue,
            };
            let key =
                SelfKey {
                    erk: erk.bytes.as_slice().try_into().map_err(|_| {
                        KeyVaultError::WrongLength {
                            at: erk.at.clone(),
                            what: erk.what.clone(),
                            got: erk.bytes.len(),
                            want: 0x20,
                        }
                    })?,
                    riv: riv.bytes.as_slice().try_into().map_err(|_| {
                        KeyVaultError::WrongLength {
                            at: riv.at.clone(),
                            what: riv.what.clone(),
                            got: riv.bytes.len(),
                            want: 0x10,
                        }
                    })?,
                };
            self.add_keyset(kind, Some(&label), key, erk.at)?;
        }
        Ok(self.vault)
    }

    /// A lone keyset half is set aside: pasted key lists are full of
    /// names that read as halves (`npdrm_idps_seed`).
    fn unpaired(&mut self, only: PendingHalf, present: Part) {
        self.vault.ignore(
            only.at,
            IgnoreReason::UnpairedHalf {
                what: only.what,
                present: present.name(),
                missing: present.other().name(),
            },
        );
    }

    /// File a complete keyset: labeled by its parsed revision (APP,
    /// NPDRM) or version range (LV2) when the label is one, otherwise
    /// as an unlabeled candidate.
    pub(super) fn add_keyset(
        &mut self,
        kind: Kind,
        label: Option<&str>,
        key: SelfKey,
        at: Provenance,
    ) -> Result<(), KeyVaultError> {
        let label = label.unwrap_or("").to_string();
        match kind {
            Kind::Scepkg => {
                self.vault.push_scepkg(SelfEntry { key, label, at });
                Ok(())
            }
            Kind::Class(SelfClass::Lv2) => match lv2_label_versions(&label) {
                Some(versions) => self
                    .vault
                    .insert_lv2_labeled(versions, SelfEntry { key, label, at }),
                None => {
                    self.vault
                        .push_unlabeled(SelfClass::Lv2, SelfEntry { key, label, at });
                    Ok(())
                }
            },
            Kind::Class(class) => match parse_revision(&label) {
                Some(revision) => {
                    self.vault
                        .insert_labeled(class, revision, SelfEntry { key, label, at })
                }
                None => {
                    self.vault
                        .push_unlabeled(class, SelfEntry { key, label, at });
                    Ok(())
                }
            },
        }
    }

    /// File one half of a keyset under `label`; `strict` turns a length
    /// that fits no half into a refusal instead of an ignore entry.
    pub(super) fn add_half(
        &mut self,
        kind: Kind,
        part: Part,
        label: String,
        value: PendingHalf,
        strict: bool,
    ) -> Result<(), KeyVaultError> {
        if value.bytes.len() != part.len() {
            if strict {
                return Err(KeyVaultError::WrongLength {
                    at: value.at,
                    what: value.what,
                    got: value.bytes.len(),
                    want: part.len(),
                });
            }
            let got = value.bytes.len();
            self.vault.ignore(
                value.at,
                IgnoreReason::LengthMismatch {
                    what: value.what,
                    got,
                    want: part.len(),
                },
            );
            return Ok(());
        }
        let halves = self.pending.entry((kind, label)).or_default();
        if let Some(existing) = halves.get(&part) {
            if existing.bytes != value.bytes {
                return Err(KeyVaultError::Conflict {
                    what: value.what,
                    first: existing.at.clone(),
                    second: value.at,
                });
            }
            return Ok(());
        }
        halves.insert(part, value);
        Ok(())
    }

    pub(super) fn add_scalar(
        &mut self,
        slot: super::Slot,
        bytes: Vec<u8>,
        what: &str,
        at: Provenance,
    ) -> Result<(), KeyVaultError> {
        if bytes.len() != slot.byte_len() {
            return Err(KeyVaultError::WrongLength {
                at,
                what: what.to_string(),
                got: bytes.len(),
                want: slot.byte_len(),
            });
        }
        self.vault.set_scalar(slot, bytes, at)
    }

    pub(super) fn walk_dir(&mut self, dir: &Path, depth: usize) -> Result<(), KeyVaultError> {
        let io = |source| KeyVaultError::Io {
            path: dir.to_path_buf(),
            source,
        };
        let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
            .map_err(io)?
            .map(|e| e.map(|e| e.path()))
            .collect::<Result<_, _>>()
            .map_err(io)?;
        entries.sort();
        for path in entries {
            let hidden = path
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with('.'));
            if hidden {
                self.vault.ignore(file_at(&path), IgnoreReason::Hidden);
                continue;
            }
            if path.is_dir() {
                if depth < MAX_WALK_DEPTH {
                    self.walk_dir(&path, depth + 1)?;
                } else {
                    self.vault.ignore(
                        file_at(&path),
                        IgnoreReason::TooDeep {
                            depth: depth + 1,
                            max: MAX_WALK_DEPTH,
                        },
                    );
                }
            } else {
                self.ingest_file(&path)?;
            }
        }
        Ok(())
    }

    pub(super) fn ingest_file(&mut self, path: &Path) -> Result<(), KeyVaultError> {
        let io = |source| KeyVaultError::Io {
            path: path.to_path_buf(),
            source,
        };
        let at = file_at(path);
        if let Some(reason) = skip_reason(&extension_of(path)) {
            self.vault.ignore(at, reason);
            return Ok(());
        }
        let len = std::fs::metadata(path).map_err(io)?.len();
        if len > MAX_KEY_FILE_BYTES {
            self.vault.ignore(at, IgnoreReason::TooLarge { bytes: len });
            return Ok(());
        }
        let bytes = std::fs::read(path).map_err(io)?;
        self.ingest_bytes(path, &bytes)
    }

    pub(super) fn ingest_bytes(&mut self, path: &Path, bytes: &[u8]) -> Result<(), KeyVaultError> {
        let at = file_at(path);
        let extension = extension_of(path);
        // The host reads a dotted version in a per-key name
        // (`lv2-key-3.60-3.61`) as a numeric extension; the name is whole.
        let name_of = if !extension.is_empty() && extension.bytes().all(|b| b.is_ascii_digit()) {
            Path::file_name
        } else {
            Path::file_stem
        };
        let stem = name_of(path)
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();
        self.vault.note_source(path);
        if extension_of(path) == "toml" {
            return self.ingest_toml(at, bytes);
        }
        // A per-key file holds one value and nothing else; anything
        // with another shape is a keyfile to read line by line.
        let value = value_bytes(bytes);
        let one_value = matches!(value.len(), 0x10 | 0x20 | 0x40);
        let by_name = if one_value {
            classify_name(&stem, Some(value.len()))
        } else {
            NameClass::Unknown
        };
        match by_name {
            NameClass::Scalar(slot) => self.add_scalar(slot, value, &stem, at),
            NameClass::Half { kind, part, label } => self.add_half(
                kind,
                part,
                label,
                PendingHalf {
                    bytes: value,
                    what: stem,
                    at,
                },
                true,
            ),
            NameClass::Unknown => match std::str::from_utf8(bytes) {
                Ok(text) => {
                    // A file that adds nothing -- no key, no half, no
                    // set-aside entry -- still leaves one line in the
                    // report, so "read and empty" is told apart from
                    // "read and placed".
                    let before = (self.vault.summary(), self.pending.len());
                    self.ingest_text(path, text)?;
                    if before == (self.vault.summary(), self.pending.len()) {
                        self.vault.ignore(at, IgnoreReason::NothingFound);
                    }
                    Ok(())
                }
                Err(_) => {
                    self.vault.ignore(at, IgnoreReason::NotText);
                    Ok(())
                }
            },
        }
    }

    fn ingest_text(&mut self, path: &Path, text: &str) -> Result<(), KeyVaultError> {
        let mut block: Option<Block> = None;
        // The nearest preceding prose line names a bare value beneath
        // it, the way a wiki heading sits over its key.
        let mut last_prose: Option<String> = None;
        for (index, raw_line) in text.lines().enumerate() {
            let line_no = index + 1;
            let at = Provenance::at(path, line_no);
            let line = strip_comment(raw_line).trim();
            if line.is_empty() {
                continue;
            }
            if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
                if let Some(done) = block.take() {
                    self.flush_block(path, done)?;
                }
                block = Some(Block {
                    name: name.trim().to_string(),
                    line: line_no,
                    props: BTreeMap::new(),
                });
                continue;
            }
            if let Some(b) = block.as_mut() {
                if let Some((prop, value)) = split_property(line) {
                    let prop = prop.to_ascii_lowercase();
                    if let Some((existing, first_line)) = b.props.get(&prop) {
                        if !existing.eq_ignore_ascii_case(value) {
                            return Err(KeyVaultError::Conflict {
                                what: format!("[{}] {prop}", redacted_name(&b.name)),
                                first: Provenance::at(path, *first_line),
                                second: at,
                            });
                        }
                        continue;
                    }
                    b.props.insert(prop, (value.to_string(), line_no));
                    continue;
                }
                // Anything else ends the block: the wiki interleaves
                // prose between keysets.
                if let Some(done) = block.take() {
                    self.flush_block(path, done)?;
                }
            }
            if let Some(row) = table_row(line) {
                self.add_table_row(row, at)?;
                last_prose = None;
                continue;
            }
            if let Some((name, value)) = named_value(line) {
                self.add_named(name, value, at)?;
                last_prose = None;
                continue;
            }
            if let Some(bytes) = bare_value(line) {
                let heading = last_prose.take();
                match heading
                    .as_deref()
                    .map(|n| classify_name(n, Some(bytes.len())))
                {
                    Some(NameClass::Scalar(slot)) => {
                        let what = heading.unwrap_or_default();
                        self.add_scalar(slot, bytes, &what, at)?;
                    }
                    _ => self
                        .vault
                        .ignore(at, IgnoreReason::UnnamedValue { bytes: bytes.len() }),
                }
                continue;
            }
            // A line no grammar placed that still carries a key-sized
            // hex token (a row with the RIV before the ERK, a value
            // trailing prose) must not become the heading over the
            // next bare value.
            if let Some(bytes) = stray_key_token(line) {
                self.vault.ignore(at, IgnoreReason::UnnamedValue { bytes });
                continue;
            }
            last_prose = Some(line.to_string());
        }
        if let Some(done) = block.take() {
            self.flush_block(path, done)?;
        }
        Ok(())
    }

    fn add_named(&mut self, name: &str, value: &str, at: Provenance) -> Result<(), KeyVaultError> {
        let shown = redacted_name(name);
        let bytes = decode_named_hex(&at, &shown, value)?;
        match classify_name(name, Some(bytes.len())) {
            NameClass::Scalar(slot) => self.add_scalar(slot, bytes, &shown, at),
            NameClass::Half { kind, part, label } => self.add_half(
                kind,
                part,
                label,
                PendingHalf {
                    bytes,
                    what: shown,
                    at,
                },
                false,
            ),
            NameClass::Unknown => {
                self.vault.ignore(
                    at,
                    IgnoreReason::UnrecognizedName {
                        name: redacted_name(name),
                    },
                );
                Ok(())
            }
        }
    }

    fn add_table_row(&mut self, row: TableRow, at: Provenance) -> Result<(), KeyVaultError> {
        let key = SelfKey {
            erk: row.erk,
            riv: row.riv,
        };
        match row.kind {
            Some(kind) => self.add_keyset(kind, row.label.as_deref(), key, at),
            None => {
                // The first column is the ERK itself on a row with no
                // type column, so it is redacted like any name.
                self.vault.ignore(
                    at,
                    IgnoreReason::UnusedKeyset {
                        name: redacted_name(&row.first),
                    },
                );
                Ok(())
            }
        }
    }

    fn flush_block(&mut self, path: &Path, block: Block) -> Result<(), KeyVaultError> {
        let at = Provenance::at(path, block.line);
        let prop = |names: &[&str]| -> Option<(String, usize)> {
            names.iter().find_map(|n| block.props.get(*n).cloned())
        };
        let erk = prop(&["erk", "key"]);
        let riv = prop(&["riv", "iv"]);
        let self_type = prop(&["self_type", "selftype"]).map(|(v, _)| v.to_ascii_uppercase());
        let ty = prop(&["type"]).map(|(v, _)| v.to_ascii_uppercase());
        let revision = prop(&["revision", "key_revision", "sdk_type"]).map(|(v, _)| v);
        let version = prop(&["version"]).map(|(v, _)| v);
        let name_words = words(&block.name);
        let has_word = |w: &str| name_words.iter().any(|n| n == w);
        let shown = redacted_name(&block.name);

        let by_name = || {
            if has_word("npdrm") || has_word("np") || has_word("drm") {
                Some(Kind::Class(SelfClass::Npdrm))
            } else if has_word("app") || has_word("appldr") {
                Some(Kind::Class(SelfClass::App))
            } else if has_word("lv2") {
                Some(Kind::Class(SelfClass::Lv2))
            } else if has_word("pkg") || has_word("scepkg") || has_word("spkg") {
                Some(Kind::Scepkg)
            } else {
                None
            }
        };
        // scetool files its loose keys (klicensees, the NP title-id
        // key) as `type=OTHER`; only a SELF or PKG block is a keyset,
        // so an OTHER block is never read as one by its name.
        let kind = match self_type.as_deref() {
            Some("APP") => Some(Kind::Class(SelfClass::App)),
            Some("NPDRM") => Some(Kind::Class(SelfClass::Npdrm)),
            Some("LV2") => Some(Kind::Class(SelfClass::Lv2)),
            Some(_) => None,
            None => match ty.as_deref() {
                Some("PKG") => Some(Kind::Scepkg),
                Some("RVK" | "SPP" | "OTHER") => None,
                _ => by_name(),
            },
        };
        let half_reason = |present: Part| {
            if kind.is_some() {
                IgnoreReason::UnpairedHalf {
                    what: format!("[{shown}]"),
                    present: present.name(),
                    missing: present.other().name(),
                }
            } else {
                IgnoreReason::UnusedKeyset {
                    name: shown.clone(),
                }
            }
        };

        let erk = match erk {
            Some((text, line)) => {
                let erk_at = Provenance::at(path, line);
                let bytes = decode_named_hex(&erk_at, &format!("[{shown}] erk"), &text)?;
                Some((bytes, erk_at))
            }
            None => None,
        };
        let riv = match riv {
            Some((text, line)) => {
                let riv_at = Provenance::at(path, line);
                let bytes = decode_named_hex(&riv_at, &format!("[{shown}] riv"), &text)?;
                Some((bytes, riv_at))
            }
            None => None,
        };
        let ((erk_bytes, erk_at), (riv_bytes, riv_at)) = match (erk, riv) {
            (Some(erk), Some(riv)) => (erk, riv),
            (Some((bytes, erk_at)), None) => {
                // A lone key: the NPDRM scalar keysets scetool names
                // `[NP_klic_free]` / `[NP_klic_key]`, or any alias.
                if let NameClass::Scalar(slot) = classify_name(&block.name, Some(bytes.len())) {
                    return self.add_scalar(slot, bytes, &shown, erk_at);
                }
                self.vault.ignore(at, half_reason(Part::Erk));
                return Ok(());
            }
            (None, Some(_)) => {
                self.vault.ignore(at, half_reason(Part::Riv));
                return Ok(());
            }
            (None, None) => {
                self.vault
                    .ignore(at, IgnoreReason::UnusedKeyset { name: shown });
                return Ok(());
            }
        };

        let Some(kind) = kind else {
            self.vault
                .ignore(at, IgnoreReason::UnusedKeyset { name: shown });
            return Ok(());
        };
        let key = SelfKey {
            erk: erk_bytes
                .as_slice()
                .try_into()
                .map_err(|_| KeyVaultError::WrongLength {
                    at: erk_at.clone(),
                    what: format!("[{shown}] erk"),
                    got: erk_bytes.len(),
                    want: 0x20,
                })?,
            riv: riv_bytes
                .as_slice()
                .try_into()
                .map_err(|_| KeyVaultError::WrongLength {
                    at: riv_at,
                    what: format!("[{shown}] riv"),
                    got: riv_bytes.len(),
                    want: 0x10,
                })?,
        };
        let label = match (kind, revision, version) {
            // The firmware versions an LV2 keyset opens label it: the
            // `version=` word, or the block's own name (`[lv2-3.55]`);
            // its `revision=` names no key.
            (Kind::Class(SelfClass::Lv2), _, Some(v)) => match Lv2Versions::parse(&v) {
                Some(_) => v,
                None => return Err(KeyVaultError::BadVersion { at, value: v }),
            },
            (Kind::Class(SelfClass::Lv2), _, None) => block.name.clone(),
            (_, Some(r), _) => match parse_revision(&r) {
                Some(_) => r,
                // scetool keeps a `revision=8000` keyset for debug
                // SELFs. Bit 15 of the SCE header's revision word marks
                // an unencrypted debug image, which
                // `sce::decrypt_self_to_elf` refuses before the vault is
                // consulted, so the keyset is never used.
                None if is_debug_revision(&r) => {
                    self.vault
                        .ignore(at, IgnoreReason::UnusedKeyset { name: shown });
                    return Ok(());
                }
                None => return Err(KeyVaultError::BadRevision { at, value: r }),
            },
            (_, None, _) => block.name.clone(),
        };
        self.add_keyset(kind, Some(&label), key, at)
    }
}

/// The firmware versions an LV2 label names: the label itself
/// (`3.60-3.61`, a `version=` word), or a block or file name with the
/// class word in front (`lv2-3.55`, `lv2-3.60-3.61`).
fn lv2_label_versions(label: &str) -> Option<Lv2Versions> {
    Lv2Versions::parse(label).or_else(|| {
        let rest: Vec<String> = words(label).into_iter().filter(|w| w != "lv2").collect();
        Lv2Versions::parse(&rest.join("-"))
    })
}

/// A revision word with the debug bit set, written as `8000` or
/// `0x8000`.
fn is_debug_revision(text: &str) -> bool {
    let t = text.trim().to_ascii_lowercase();
    let digits = t.strip_prefix("0x").unwrap_or(&t);
    u16::from_str_radix(digits, 16).is_ok_and(|r| r & 0x8000 != 0)
}

/// The largest key-sized hex token on a line, when it has one.
fn stray_key_token(line: &str) -> Option<usize> {
    let tokens: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || matches!(c, '|' | ',' | ';'))
        .map(|t| t.trim_matches(|c: char| matches!(c, '"' | '\'' | '(' | ')' | '[' | ']')))
        .filter(|t| !t.is_empty())
        .collect();
    [0x40, 0x20, 0x10]
        .into_iter()
        .find(|n| tokens.iter().any(|t| is_hex_token(t, *n)))
}

fn file_at(path: &Path) -> Provenance {
    Provenance::file(path)
}

/// A name as the report may show it: every hex run of eight or more
/// digits becomes `<hex>` and the result is cut to one line's worth,
/// so a name that swallowed a neighbouring value (a pasted page
/// glues `name: HEX name: HEX` onto one line) never echoes it.
fn redacted_name(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() >= 8 {
            out.push_str("<hex>");
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for c in name.chars() {
        if c.is_ascii_hexdigit() {
            run.push(c);
        } else {
            flush(&mut run, &mut out);
            out.push(c);
        }
    }
    flush(&mut run, &mut out);
    const MAX: usize = 72;
    if out.chars().count() > MAX {
        let cut: String = out.chars().take(MAX).collect();
        format!("{cut}...")
    } else {
        out
    }
}

pub(super) fn decode_named_hex(
    at: &Provenance,
    what: &str,
    text: &str,
) -> Result<Vec<u8>, KeyVaultError> {
    decode_hex(text).map_err(|source| KeyVaultError::BadHex {
        at: at.clone(),
        what: what.to_string(),
        source,
    })
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// Extensions the directory walk never reads.
fn skip_reason(ext: &str) -> Option<IgnoreReason> {
    match ext {
        "rap" => Some(IgnoreReason::RapFile),
        "zip" | "7z" | "rar" | "gz" | "tar" | "pdf" | "png" | "jpg" | "jpeg" | "gif" | "html"
        | "htm" | "iso" | "pkg" | "pup" | "self" | "sprx" | "prx" | "elf" | "exe" | "dll"
        | "so" | "dylib" | "bak" | "tmp" => Some(IgnoreReason::UnsupportedFile {
            extension: ext.to_string(),
        }),
        _ => None,
    }
}

fn strip_comment(line: &str) -> &str {
    let t = line.trim_start();
    if t.starts_with('#') || t.starts_with(';') || t.starts_with("//") {
        return "";
    }
    // A trailing comment is only one that is set off by whitespace,
    // so a `sc_key::x` style name keeps its `::`.
    for marker in [" #", "\t#", " //", "\t//"] {
        if let Some(i) = line.find(marker) {
            return &line[..i];
        }
    }
    line
}

/// `prop=value` inside a `[section]`; the property is a bare word.
fn split_property(line: &str) -> Option<(&str, &str)> {
    let (prop, value) = line.split_once('=')?;
    let prop = prop.trim();
    if prop.is_empty() || !prop.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return None;
    }
    Some((prop, value.trim()))
}

/// `name: HEX` / `name = HEX`: the last token is the value, everything
/// before the separator is the name. A value that is itself
/// colon-separated (`name: aa:bb:...`) is read from the first
/// separator instead, since the last one sits inside the value.
fn named_value(line: &str) -> Option<(&str, &str)> {
    [
        line.rsplit_once('='),
        line.rsplit_once(':'),
        line.split_once('='),
        line.split_once(':'),
    ]
    .into_iter()
    .flatten()
    .find_map(|(name, value)| {
        let value = value.trim();
        let name = name.trim().trim_end_matches([':', '=']).trim();
        if value.is_empty() || !name.chars().any(|c| c.is_ascii_alphabetic()) {
            return None;
        }
        decode_hex(value)
            .ok()
            .filter(|b| b.len() >= 16)
            .map(|_| (name, value))
    })
}

/// A line that is one hex value of at least 16 bytes and nothing else.
fn bare_value(line: &str) -> Option<Vec<u8>> {
    let candidate = line.trim();
    let plausible = candidate.bytes().all(|b| {
        b.is_ascii_hexdigit()
            || matches!(
                b,
                b' ' | b'\t' | b',' | b':' | b'x' | b'X' | b'{' | b'}' | b';'
            )
    });
    if !plausible {
        return None;
    }
    decode_hex(candidate).ok().filter(|b| b.len() >= 16)
}

/// One pasted key-table row: a 32-byte ERK token immediately followed
/// by a 16-byte RIV token, with the class read off the other columns.
struct TableRow {
    kind: Option<Kind>,
    /// `0x..` as written (`sd-0x..` for a row badged `SD`, which files
    /// as a candidate beside the plain row at the same revision), or the
    /// version range of an LV2 row.
    label: Option<String>,
    first: String,
    erk: [u8; 0x20],
    riv: [u8; 0x10],
}

fn table_row(line: &str) -> Option<TableRow> {
    let tokens: Vec<&str> = line
        .split(|c: char| c.is_whitespace() || c == '|')
        .filter(|t| !t.is_empty())
        .collect();
    if tokens.len() < 3 {
        return None;
    }
    let erk_index = tokens.iter().position(|t| is_hex_token(t, 0x20))?;
    let riv_token = tokens.get(erk_index + 1)?;
    if !is_hex_token(riv_token, 0x10) {
        return None;
    }
    let erk = decode_hex(tokens[erk_index]).ok()?;
    let riv = decode_hex(riv_token).ok()?;
    let mut kind = None;
    let mut revision = None;
    let mut versions = None;
    let mut np_marker = false;
    let mut sd_marker = false;
    for (i, t) in tokens.iter().enumerate() {
        if i == erk_index || i == erk_index + 1 {
            continue;
        }
        match normalized(t).as_str() {
            "app" | "appldr" => kind = kind.or(Some(Kind::Class(SelfClass::App))),
            "npdrm" => kind = kind.or(Some(Kind::Class(SelfClass::Npdrm))),
            // `lv2ldr` rows stay unplaced; see `names::classify_name`.
            "lv2" => kind = kind.or(Some(Kind::Class(SelfClass::Lv2))),
            "pkg" | "spkg" | "scepkg" => kind = kind.or(Some(Kind::Scepkg)),
            "np" => np_marker = true,
            "sd" => sd_marker = true,
            _ => {}
        }
        // The revision column precedes the ERK; the `0x..` after the
        // RIV is the curve type, which would otherwise label the row.
        if i < erk_index && revision.is_none() && t.starts_with("0x") {
            revision = parse_revision(t);
        }
        // The version-range column (`3.60-3.61`) labels an LV2 row.
        if i < erk_index && versions.is_none() && t.contains('.') {
            versions = Lv2Versions::parse(t);
        }
    }
    if np_marker && matches!(kind, None | Some(Kind::Class(SelfClass::App))) {
        kind = Some(Kind::Class(SelfClass::Npdrm));
    }
    let label = match kind {
        Some(Kind::Class(SelfClass::Lv2)) => versions.map(|v| v.to_string()),
        _ => revision.map(|r| {
            if sd_marker {
                format!("sd-0x{r:04x}")
            } else {
                format!("0x{r:04x}")
            }
        }),
    };
    Some(TableRow {
        kind,
        label,
        first: tokens[0].to_string(),
        erk: erk.as_slice().try_into().ok()?,
        riv: riv.as_slice().try_into().ok()?,
    })
}
