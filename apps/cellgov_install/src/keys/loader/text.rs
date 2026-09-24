//! The text grammar: scetool blocks, named lines, table rows and the constructor catalog.

use std::collections::BTreeMap;
use std::path::Path;

use crate::keys::hex::{decode_hex, is_hex_token};
use crate::keys::names::{classify_name, parse_revision, words, Kind, NameClass, Part};
use crate::keys::{
    version_label, CryptoMaterial, IgnoreReason, KeyVaultError, Lv2Versions, Provenance, SelfClass,
    SelfKey,
};

use super::builder::{Loader, PendingHalf};
use super::parse::{
    bare_value, constructor_version_range, named_value, quoted_hex_values, split_property,
    strip_comment, table_row, TableRow,
};

/// One `[section]` of a text keyfile while its properties accumulate.
struct Block {
    name: String,
    line: usize,
    props: BTreeMap<String, (String, usize)>,
}

impl Loader {
    pub(super) fn ingest_text(&mut self, path: &Path, text: &str) -> Result<(), KeyVaultError> {
        if self.ingest_constructor_catalog(path, text)? {
            return Ok(());
        }
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

    /// Import constructor-style key catalog rows without treating their
    /// values as prose. This preserves all binary components even when no
    /// CellGov decrypt path consumes that key class yet.
    fn ingest_constructor_catalog(
        &mut self,
        path: &Path,
        text: &str,
    ) -> Result<bool, KeyVaultError> {
        if !text.contains(".emplace_back(") {
            return Ok(false);
        }
        let mut kind = String::from("catalog");
        let mut lines = text.lines().enumerate().peekable();
        let mut found = false;
        while let Some((index, raw)) = lines.next() {
            let line = raw.trim();
            if let Some(start) = line.find("LoadSelf") {
                let rest = &line[start + "LoadSelf".len()..];
                if let Some(end) = rest.find("Keys") {
                    kind = rest[..end].to_ascii_lowercase();
                }
            }
            if !line.contains(".emplace_back(") {
                continue;
            }
            if let Some(receiver) = line.split(".emplace_back(").next() {
                let receiver = receiver.trim();
                if let Some(name) = receiver
                    .strip_prefix("sk_")
                    .and_then(|v| v.strip_suffix("_arr"))
                {
                    kind = name.to_ascii_lowercase();
                }
            }
            let mut row = line.to_string();
            while !row.contains(");") {
                let Some((_, next)) = lines.next() else { break };
                row.push(' ');
                row.push_str(next.trim());
            }
            let strings = quoted_hex_values(&row);
            if strings.is_empty() {
                continue;
            }
            let names = ["erk", "riv", "pub", "priv"];
            let components = strings
                .into_iter()
                .enumerate()
                .map(|(component, value)| {
                    let name = names.get(component).copied().unwrap_or("component");
                    let name = if component < names.len() {
                        name.to_string()
                    } else {
                        format!("{name}_{component}")
                    };
                    Ok((
                        name.clone(),
                        decode_named_hex(&Provenance::at(path, index + 1), &name, &value)?,
                    ))
                })
                .collect::<Result<Vec<_>, KeyVaultError>>()?;
            let version_range = constructor_version_range(&row);
            if kind == "lv2" && components.len() >= 2 {
                let (erk, riv) = (&components[0].1, &components[1].1);
                if let (Ok(erk), Ok(riv), Some((start, end))) = (
                    erk.as_slice().try_into(),
                    riv.as_slice().try_into(),
                    version_range,
                ) {
                    self.add_keyset(
                        Kind::Class(SelfClass::Lv2),
                        Some(&format!("{}-{}", version_label(start), version_label(end))),
                        SelfKey { erk, riv },
                        Provenance::at(path, index + 1),
                    )?;
                }
            }
            self.vault.material.push((
                CryptoMaterial {
                    kind: kind.clone(),
                    label: version_range.map_or_else(
                        || format!("row-{}", index + 1),
                        |(start, end)| format!("{start:016x}-{end:016x}"),
                    ),
                    components,
                },
                Provenance::at(path, index + 1),
            ));
            found = true;
        }
        Ok(found)
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
pub(super) fn lv2_label_versions(label: &str) -> Option<Lv2Versions> {
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

pub(super) fn file_at(path: &Path) -> Provenance {
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

pub(in crate::keys) fn decode_named_hex(
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
