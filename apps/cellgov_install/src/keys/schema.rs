//! The `keys.toml` schema: what `keys import` writes and what
//! [`KeyVault::to_toml`] renders.

use std::collections::BTreeMap;

use serde::de::DeserializeOwned;
use serde::Deserialize;

use super::hex::hex;
use super::loader::{Loader, PendingHalf};
use super::names::{classify_name, parse_revision, Kind, NameClass};
use super::{
    CryptoMaterial, KeyVault, KeyVaultError, Lv2Versions, Provenance, SelfClass, SelfKey, Slot,
};

/// One `[[app]]` / `[[npdrm]]` / `[[lv2]]` / `[[scepkg]]` table.
///
/// `revision` labels an APP or NPDRM keyset and `version` (a
/// [`Lv2Versions`] label) an LV2 one; the loader refuses the other
/// field on each.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlSelfKey {
    revision: Option<TomlRevision>,
    version: Option<String>,
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

struct TomlVault {
    scepkg: Vec<TomlSelfKey>,
    app: Vec<TomlSelfKey>,
    npdrm: Vec<TomlSelfKey>,
    lv2: Vec<TomlSelfKey>,
    material: Vec<TomlMaterial>,
    scalars: BTreeMap<String, toml::Value>,
}

impl TomlVault {
    /// Separates known table arrays before the remaining top-level values
    /// become scalar candidates.
    fn parse(text: &str) -> Result<Self, toml::de::Error> {
        let mut values: BTreeMap<String, toml::Value> = toml::from_str(text)?;
        Ok(Self {
            scepkg: take_table_array(&mut values, "scepkg")?,
            app: take_table_array(&mut values, "app")?,
            npdrm: take_table_array(&mut values, "npdrm")?,
            lv2: take_table_array(&mut values, "lv2")?,
            material: take_table_array(&mut values, "material")?,
            scalars: values,
        })
    }
}

fn take_table_array<T: DeserializeOwned>(
    values: &mut BTreeMap<String, toml::Value>,
    name: &str,
) -> Result<Vec<T>, toml::de::Error> {
    values
        .remove(name)
        .map(toml::Value::try_into)
        .transpose()
        .map(|rows| rows.unwrap_or_default())
}

/// A lossless record for key material with no current decrypt-path consumer.
#[derive(Deserialize)]
struct TomlMaterial {
    kind: String,
    #[serde(default)]
    label: String,
    #[serde(flatten)]
    components: BTreeMap<String, String>,
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
        let parsed = TomlVault::parse(text).map_err(|source| KeyVaultError::Toml {
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
            (Kind::Class(SelfClass::Lv2), parsed.lv2),
        ] {
            for entry in entries {
                let key = SelfKey {
                    erk: fixed(&at, &format!("[[{kind}]] erk"), &entry.erk)?,
                    riv: fixed(&at, &format!("[[{kind}]] riv"), &entry.riv)?,
                };
                // The `[[scepkg]]` schema has no `version`, so the loader
                // refuses one the way the parser refuses a field it does
                // not know.
                let wrong_label = |expected, found| match kind {
                    Kind::Class(class) => KeyVaultError::WrongLabelKind {
                        at: at.clone(),
                        class,
                        expected,
                        found,
                    },
                    Kind::Scepkg => toml_refusal(
                        &at,
                        format!("[[scepkg]] takes no {found}; the field applies to [[lv2]] only"),
                    ),
                };
                let label = match (kind, entry.revision, entry.version) {
                    (Kind::Class(SelfClass::Lv2), Some(_), _) => {
                        return Err(wrong_label("version", "revision"))
                    }
                    (Kind::Class(SelfClass::Lv2), None, Some(v)) => {
                        if Lv2Versions::parse(&v).is_none() {
                            return Err(KeyVaultError::BadVersion { at, value: v });
                        }
                        v
                    }
                    (_, _, Some(_)) => return Err(wrong_label("revision", "version")),
                    (_, Some(TomlRevision::Int(i)), None) => {
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
                    (_, Some(TomlRevision::Text(s)), None) => {
                        if parse_revision(&s).is_none() {
                            return Err(KeyVaultError::BadRevision { at, value: s });
                        }
                        s
                    }
                    (_, None, None) => entry.label.unwrap_or_default(),
                };
                self.add_keyset(kind, Some(&label), key, at.clone())?;
            }
        }
        for entry in parsed.material {
            if entry.kind.trim().is_empty()
                || !entry
                    .kind
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
            {
                return Err(toml_refusal(
                    &at,
                    "[[material]] kind must be an ASCII identifier".to_string(),
                ));
            }
            let mut components = Vec::new();
            for (name, value) in entry.components {
                if name == "kind" || name == "label" {
                    continue;
                }
                if !name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                {
                    return Err(toml_refusal(
                        &at,
                        format!("[[material]] component {name:?} must be an ASCII identifier"),
                    ));
                }
                components.push((
                    name.clone(),
                    super::loader::decode_named_hex(&at, &name, &value)?,
                ));
            }
            if components.is_empty() {
                return Err(toml_refusal(
                    &at,
                    "[[material]] needs at least one component".to_string(),
                ));
            }
            self.vault.material.push((
                CryptoMaterial {
                    kind: entry.kind,
                    label: entry.label,
                    components,
                },
                at.clone(),
            ));
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
        for (versions, entry) in &self.lv2.labeled {
            out.push_str(&format!("\n[[lv2]]\nversion = \"{versions}\"\n"));
            push_self_key_body(&mut out, &entry.key);
        }
        for entry in &self.lv2.unlabeled {
            out.push_str(&format!(
                "\n[[lv2]]\nlabel = {}\n",
                toml_string(&entry.label)
            ));
            push_self_key_body(&mut out, &entry.key);
        }
        for (material, _) in &self.material {
            out.push_str(&format!(
                "\n[[material]]\nkind = {}\nlabel = {}\n",
                toml_string(&material.kind),
                toml_string(&material.label)
            ));
            for (name, bytes) in &material.components {
                out.push_str(&format!("{name} = \"{}\"\n", hex(bytes)));
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
