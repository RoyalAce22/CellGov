//! What a key name denotes: the aliases each scalar slot answers to,
//! the `<class>-<part>-<suffix>` shape of a keyset half, and the
//! revision a label may carry.

use std::fmt;

use super::{SelfClass, Slot};

/// `name` normalized for alias lookup: lowercase, alphanumerics only.
pub(super) fn normalized(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// `name` split into lowercase alphanumeric words.
pub(super) fn words(name: &str) -> Vec<String> {
    name.split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// One half of a keyset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Part {
    Erk,
    Riv,
}

impl Part {
    pub(super) const fn name(self) -> &'static str {
        match self {
            Part::Erk => "erk",
            Part::Riv => "riv",
        }
    }

    pub(super) const fn len(self) -> usize {
        match self {
            Part::Erk => 0x20,
            Part::Riv => 0x10,
        }
    }

    pub(super) const fn other(self) -> Part {
        match self {
            Part::Erk => Part::Riv,
            Part::Riv => Part::Erk,
        }
    }
}

/// Which table a keyset belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Kind {
    Scepkg,
    Class(SelfClass),
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Kind::Scepkg => f.write_str("scepkg"),
            Kind::Class(c) => write!(f, "{c}"),
        }
    }
}

/// What a key name (file stem, `name: value` line, TOML field) denotes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum NameClass {
    Scalar(Slot),
    /// One half of a keyset, filed under `label` until its other half
    /// arrives.
    Half {
        kind: Kind,
        part: Part,
        label: String,
    },
    Unknown,
}

/// `value_len` disambiguates names that mean different slots at
/// different lengths (`pkg-key`: 16 bytes is the PKG AES key, 32 the
/// SCE package ERK).
pub(super) fn classify_name(name: &str, value_len: Option<usize>) -> NameClass {
    let joined = normalized(name);
    let scalar = match joined.as_str() {
        "pupkey" | "puphmac" | "puphmackey" | "pup" | "ps3puphmac" | "ps3pupkey"
        | "pupupdatekey" | "pupkeyhmac" => Some(Slot::PupHmac),
        "pkgaeskey" | "pkgaes" | "npdrmpkgps3aeskey" | "npdrmpkgaeskey" | "npdrmpkgkey"
        | "ps3aeskey" | "retailpkgkey" | "pkgcontentkey" | "ps3pkgaeskey" | "pkgps3aeskey" => {
            Some(Slot::PkgAes)
        }
        "npklickey" | "klickey" | "ps3klicdeckey" | "klicdeckey" | "klicaeskey"
        | "npdrmklickey" | "klicenseekey" | "npklic" | "klic" => Some(Slot::NpKlicKey),
        "npklicfree"
        | "klicfree"
        | "klicps3free"
        | "ps3klicfreekey"
        | "ps3klicfree"
        | "freeklic"
        | "npdrmfreeklic"
        | "ps3drmfreeklicensee"
        | "klicenseefree"
        | "npdrmklicfree"
        | "drmfreeklicensee" => Some(Slot::NpKlicFree),
        "rapkey" | "rapinitkey" | "rapinitialkey" | "rapinit" | "rapaeskey" => Some(Slot::RapKey),
        "rappbox" | "pbox" => Some(Slot::RapPbox),
        "rape1" | "e1" => Some(Slot::RapE1),
        "rape2" | "e2" => Some(Slot::RapE2),
        "pkgkey" | "pkg" => match value_len {
            Some(0x20) => return half(Kind::Scepkg, Part::Erk, String::new()),
            _ => Some(Slot::PkgAes),
        },
        _ => None,
    };
    if let Some(slot) = scalar {
        return NameClass::Scalar(slot);
    }
    match joined.as_str() {
        "scepkgerk" | "scepkgkey" | "pkgerk" => {
            return half(Kind::Scepkg, Part::Erk, String::new())
        }
        "scepkgriv" | "scepkgiv" | "pkgriv" | "pkgiv" => {
            return half(Kind::Scepkg, Part::Riv, String::new())
        }
        _ => {}
    }
    // `<class>-<part>-<suffix>` in any word order: the suffix is
    // whatever is left once the class and part words are taken. An
    // LV2 suffix is a version range whose `.` and `-` the word split
    // dropped; `Lv2Versions::parse` reads the dashed form back.
    let mut kind: Option<Kind> = None;
    let mut part: Option<Part> = None;
    let mut rest: Vec<String> = Vec::new();
    for w in words(name) {
        match w.as_str() {
            "app" | "appldr" if kind.is_none() => kind = Some(Kind::Class(SelfClass::App)),
            "npdrm" | "np" | "drm" if kind.is_none() => kind = Some(Kind::Class(SelfClass::Npdrm)),
            // A pasted `lv2ldr <range> ERK RIV` row is one of lv0's
            // loader keys, which open the loader binary and no kernel,
            // so `lv2ldr` is no alias.
            "lv2" if kind.is_none() => kind = Some(Kind::Class(SelfClass::Lv2)),
            "pkg" | "scepkg" | "spkg" if kind.is_none() => kind = Some(Kind::Scepkg),
            "key" | "erk" if part.is_none() => part = Some(Part::Erk),
            "iv" | "riv" if part.is_none() => part = Some(Part::Riv),
            "pub" | "priv" | "ctype" | "curvetype" | "public" | "private" => {
                return NameClass::Unknown
            }
            _ => rest.push(w),
        }
    }
    match (kind, part) {
        (Some(kind), Some(part)) => half(kind, part, rest.join("-")),
        _ => NameClass::Unknown,
    }
}

fn half(kind: Kind, part: Part, label: String) -> NameClass {
    NameClass::Half { kind, part, label }
}

/// A revision written as `0A`, `0x0A`, or `rev0A`: one or two hex
/// digits bare, up to four behind an explicit `0x`. A bare `r` is not
/// a prefix: `red` and `rff` are words, not revisions.
pub(super) fn parse_revision(text: &str) -> Option<u16> {
    let t = text.trim().to_ascii_lowercase();
    let t = t.strip_prefix("rev").unwrap_or(&t);
    let (digits, explicit) = match t.strip_prefix("0x") {
        Some(d) => (d, true),
        None => (t, false),
    };
    let max_digits = if explicit { 4 } else { 2 };
    if digits.is_empty()
        || digits.len() > max_digits
        || !digits.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    u16::from_str_radix(digits, 16)
        .ok()
        .filter(|r| *r <= 0x7FFF)
}
