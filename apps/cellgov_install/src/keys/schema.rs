//! The `keys.toml` schema: what `keys import` writes and what
//! [`KeyVault::to_toml`] renders.

use std::collections::BTreeMap;

use serde::Deserialize;

use super::hex::hex;
use super::loader::{Loader, PendingHalf};
use super::names::{classify_name, parse_revision, Kind, NameClass};
use super::{KeyVault, KeyVaultError, Provenance, SelfClass, SelfKey, Slot};

/// One `[[app]]` / `[[npdrm]]` / `[[scepkg]]` table.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlSelfKey {
    revision: Option<TomlRevision>,
    label: Option<String>,
    erk: String,
    riv: String,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum TomlRevision {
    Int(i64),
    Text(String),
}

#[derive(Deserialize)]
struct TomlVault {
    #[serde(default)]
    scepkg: Vec<TomlSelfKey>,
    #[serde(default)]
    app: Vec<TomlSelfKey>,
    #[serde(default)]
    npdrm: Vec<TomlSelfKey>,
    #[serde(flatten)]
    scalars: BTreeMap<String, toml::Value>,
}

impl Loader {
    pub(super) fn ingest_toml(
        &mut self,
        at: Provenance,
        bytes: &[u8],
    ) -> Result<(), KeyVaultError> {
        let text = std::str::from_utf8(bytes).map_err(|e| KeyVaultError::Toml {
            at: at.clone(),
            source: Box::new(<toml::de::Error as serde::de::Error>::custom(e)),
        })?;
        let parsed: TomlVault = toml::from_str(text).map_err(|source| KeyVaultError::Toml {
            at: at.clone(),
            source: Box::new(source),
        })?;
        for (name, value) in parsed.scalars {
            let raw = match value {
                toml::Value::String(s) => super::loader::decode_named_hex(&at, &name, &s)?,
                toml::Value::Array(items) => {
                    let mut raw = Vec::with_capacity(items.len());
                    for (index, item) in items.iter().enumerate() {
                        match item {
                            toml::Value::Integer(i) if (0..=255).contains(i) => raw.push(*i as u8),
                            toml::Value::Integer(_) => {
                                return Err(toml_refusal(
                                    &at,
                                    format!(
                                        "field {name:?}: byte array element {index} is outside 0..=255"
                                    ),
                                ))
                            }
                            other => {
                                return Err(toml_refusal(
                                    &at,
                                    format!(
                                        "field {name:?}: byte array element {index} is {}, not an integer",
                                        other.type_str()
                                    ),
                                ))
                            }
                        }
                    }
                    raw
                }
                _ if classify_name(&name, None) == NameClass::Unknown => {
                    return Err(KeyVaultError::UnknownName { at, name })
                }
                other => {
                    return Err(toml_refusal(
                        &at,
                        format!(
                            "field {name:?} must be a hex string or byte array, not {}",
                            other.type_str()
                        ),
                    ))
                }
            };
            match classify_name(&name, Some(raw.len())) {
                NameClass::Scalar(slot) => self.add_scalar(slot, raw, &name, at.clone())?,
                NameClass::Half { kind, part, label } => self.add_half(
                    kind,
                    part,
                    label,
                    PendingHalf {
                        bytes: raw,
                        what: name,
                        at: at.clone(),
                    },
                    true,
                )?,
                NameClass::Unknown => return Err(KeyVaultError::UnknownName { at, name }),
            }
        }
        for (kind, entries) in [
            (Kind::Scepkg, parsed.scepkg),
            (Kind::Class(SelfClass::App), parsed.app),
            (Kind::Class(SelfClass::Npdrm), parsed.npdrm),
        ] {
            for entry in entries {
                let key = SelfKey {
                    erk: fixed(&at, &format!("[[{kind}]] erk"), &entry.erk)?,
                    riv: fixed(&at, &format!("[[{kind}]] riv"), &entry.riv)?,
                };
                let label = match entry.revision {
                    Some(TomlRevision::Int(i)) => {
                        let revision =
                            u16::try_from(i)
                                .ok()
                                .filter(|r| *r <= 0x7FFF)
                                .ok_or_else(|| KeyVaultError::BadRevision {
                                    at: at.clone(),
                                    value: i.to_string(),
                                })?;
                        format!("0x{revision:04x}")
                    }
                    Some(TomlRevision::Text(s)) => {
                        if parse_revision(&s).is_none() {
                            return Err(KeyVaultError::BadRevision { at, value: s });
                        }
                        s
                    }
                    None => entry.label.unwrap_or_default(),
                };
                self.add_keyset(kind, Some(&label), key, at.clone())?;
            }
        }
        Ok(())
    }
}

/// A `keys.toml` refusal the parser itself would not raise.
fn toml_refusal(at: &Provenance, message: String) -> KeyVaultError {
    KeyVaultError::Toml {
        at: at.clone(),
        source: Box::new(<toml::de::Error as serde::de::Error>::custom(message)),
    }
}

fn fixed<const N: usize>(
    at: &Provenance,
    what: &str,
    text: &str,
) -> Result<[u8; N], KeyVaultError> {
    let bytes = super::loader::decode_named_hex(at, what, text)?;
    bytes
        .as_slice()
        .try_into()
        .map_err(|_| KeyVaultError::WrongLength {
            at: at.clone(),
            what: what.to_string(),
            got: bytes.len(),
            want: N,
        })
}

impl KeyVault {
    /// The vault as a `keys.toml`, the form `keys import` writes.
    #[must_use]
    pub fn to_toml(&self) -> String {
        let mut out = String::new();
        out.push_str("# CellGov key vault. Hex values; edit or re-import to change.\n\n");
        for slot in Slot::ALL {
            if let Some((bytes, _)) = self.scalars.get(&slot) {
                out.push_str(&format!("{} = \"{}\"\n", slot.name(), hex(bytes)));
            }
        }
        for entry in &self.scepkg {
            out.push_str("\n[[scepkg]]\n");
            push_self_key_body(&mut out, &entry.key);
        }
        for (class, table) in [(SelfClass::App, &self.app), (SelfClass::Npdrm, &self.npdrm)] {
            for (revision, entry) in &table.labeled {
                out.push_str(&format!("\n[[{class}]]\nrevision = 0x{revision:04x}\n"));
                push_self_key_body(&mut out, &entry.key);
            }
            for entry in &table.unlabeled {
                out.push_str(&format!(
                    "\n[[{class}]]\nlabel = {}\n",
                    toml_string(&entry.label)
                ));
                push_self_key_body(&mut out, &entry.key);
            }
        }
        out
    }
}

fn push_self_key_body(out: &mut String, key: &SelfKey) {
    out.push_str(&format!(
        "erk = \"{}\"\nriv = \"{}\"\n",
        hex(&key.erk),
        hex(&key.riv)
    ));
}

fn toml_string(s: &str) -> String {
    let escaped: String = s
        .chars()
        .flat_map(|c| match c {
            '"' => vec!['\\', '"'],
            '\\' => vec!['\\', '\\'],
            c if c.is_control() => format!("\\u{:04X}", c as u32).chars().collect(),
            c => vec![c],
        })
        .collect();
    format!("\"{escaped}\"")
}
