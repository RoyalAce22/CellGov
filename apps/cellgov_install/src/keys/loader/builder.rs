//! The vault builder: its pending keyset halves, the add methods, and the pairing at the end.

use std::collections::BTreeMap;

use crate::keys::names::{parse_revision, Kind, Part};
use crate::keys::vault::SelfEntry;
use crate::keys::{IgnoreReason, KeyVault, KeyVaultError, Provenance, SelfClass, SelfKey};

use super::text::lv2_label_versions;

/// A keyset half waiting for its other half.
pub(in crate::keys) struct PendingHalf {
    pub(in crate::keys) bytes: Vec<u8>,
    pub(in crate::keys) what: String,
    pub(in crate::keys) at: Provenance,
}

/// Accumulates one [`KeyVault`] across files, pairing keyset halves at
/// the end.
pub(in crate::keys) struct Loader {
    pub(in crate::keys) vault: KeyVault,
    pub(super) pending: BTreeMap<(Kind, String), BTreeMap<Part, PendingHalf>>,
}

impl Loader {
    pub(in crate::keys) fn new() -> Self {
        Self {
            vault: KeyVault::empty(),
            pending: BTreeMap::new(),
        }
    }

    pub(in crate::keys) fn finish(mut self) -> Result<KeyVault, KeyVaultError> {
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
    pub(in crate::keys) fn add_keyset(
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
    pub(in crate::keys) fn add_half(
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

    pub(in crate::keys) fn add_scalar(
        &mut self,
        slot: crate::keys::Slot,
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
}
