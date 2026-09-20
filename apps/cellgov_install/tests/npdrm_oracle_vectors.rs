//! NPDRM witness vectors over the operator's installed content.
//!
//! The operator pins, in `vfs/.cellgov/npdrm_oracle_vectors.toml`,
//! RAP files under the workspace `vfs/` with the klicensee each must
//! derive to, and the NPDRM EBOOT that klicensee opens.
//!
//! ```toml
//! [[vector]]
//! rap = "dev_hdd0/home/00000001/exdata/<content-id>.rap"  # relative to vfs/
//! klicensee = "<32 hex chars>"                            # what rap_to_klic derives
//! eboot = "dev_hdd0/game/<title-id>/USRDIR/EBOOT.BIN"      # optional; opens under it
//! ```
//!
//! Compiled only under `npdrm-oracle-vectors`, which declares the
//! manifest present: a missing or empty manifest fails, as does one
//! that pins no EBOOT.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use std::path::{Component, Path, PathBuf};

use cellgov_install::npdrm::{decrypt_self_to_elf_npdrm, rap_to_klic};
use serde::Deserialize;

#[path = "common/keys.rs"]
mod keys;

/// Manifest location, relative to the workspace `vfs/`.
const MANIFEST: &str = ".cellgov/npdrm_oracle_vectors.toml";

/// The operator's manifest; unknown fields are refused so a misspelt
/// `eboot` cannot silently drop a row's container proof.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    vector: Vec<Vector>,
}

/// One `[[vector]]` row.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Vector {
    /// RAP file, relative to the workspace `vfs/`.
    rap: String,
    /// Hex of the klicensee `rap_to_klic` derives from it.
    klicensee: String,
    /// NPDRM EBOOT that klicensee opens, relative to the workspace
    /// `vfs/`.
    eboot: Option<String>,
}

fn manifest_path() -> PathBuf {
    keys::workspace_vfs().join(MANIFEST)
}

/// Parse the operator's manifest.
///
/// # Panics
///
/// If the file is unreadable, does not parse, or pins no vector.
fn load() -> Vec<Vector> {
    let path = manifest_path();
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "npdrm-oracle-vectors is on but the witness manifest {} is unreadable ({e}); \
             write one, or build without the feature",
            path.display()
        )
    });
    parse(&text, &path.display().to_string())
}

/// Parse `text`; `origin` names it in diagnostics.
///
/// # Panics
///
/// If `text` does not parse as the manifest schema, pins no vector,
/// pins the same RAP twice, pins a path that is not relative under
/// `vfs/`, or pins a klicensee that is not 32 hex characters.
fn parse(text: &str, origin: &str) -> Vec<Vector> {
    let parsed: Manifest = toml::from_str(text).unwrap_or_else(|e| panic!("parse {origin}: {e}"));
    assert!(
        !parsed.vector.is_empty(),
        "{origin} pins no [[vector]], so the suite would hold nothing"
    );
    let mut seen: Vec<&str> = Vec::new();
    for v in &parsed.vector {
        assert!(
            !seen.contains(&v.rap.as_str()),
            "{origin} pins RAP {:?} more than once; delete the stale row rather than \
             deriving it twice",
            v.rap
        );
        seen.push(v.rap.as_str());
        assert_under_vfs(&v.rap, "rap", &v.rap);
        if let Some(eboot) = &v.eboot {
            assert_under_vfs(eboot, "eboot", &v.rap);
        }
        hex_to_bytes16(&v.klicensee, &v.rap);
    }
    parsed.vector
}

/// Refuse a manifest path that is not a plain relative path under the
/// workspace `vfs/`; `ctx` names the row in diagnostics.
///
/// `vfs.join` replaces the base with an absolute path and follows
/// `..` out of it, so either would witness a file outside the
/// installed content the suite claims to hold.
///
/// # Panics
///
/// If `path` is empty, absolute, or climbs with `..`.
fn assert_under_vfs(path: &str, what: &str, ctx: &str) {
    let mut named = 0usize;
    for component in Path::new(path).components() {
        match component {
            Component::Normal(_) => named += 1,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => panic!(
                "{ctx}: {what} {path:?} leaves the workspace vfs/; pin a path relative to it"
            ),
        }
    }
    assert!(
        named > 0,
        "{ctx}: {what} is empty; pin a path relative to the workspace vfs/"
    );
}

/// Decode a 32-character hex string; `ctx` names it in diagnostics.
///
/// # Panics
///
/// If the string is not exactly 32 hex characters.
fn hex_to_bytes16(s: &str, ctx: &str) -> [u8; 16] {
    assert_eq!(
        s.len(),
        32,
        "{ctx}: klicensee hex must be 32 chars, got {}",
        s.len()
    );
    let mut out = [0u8; 16];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16)
            .unwrap_or_else(|_| panic!("{ctx}: invalid hex byte at index {i} in {s:?}"));
    }
    out
}

fn read_rap(path: &Path) -> [u8; 16] {
    let rap = std::fs::read(path).unwrap_or_else(|e| panic!("read RAP {}: {e}", path.display()));
    rap.as_slice()
        .try_into()
        .unwrap_or_else(|_| panic!("RAP {} is {} bytes, expected 16", path.display(), rap.len()))
}

/// Assert that `elf` is a 64-bit big-endian PS3 ELF whose declared
/// phdr table fits within the buffer.
fn assert_is_ps3_ppc64_elf(elf: &[u8], ctx: &str) {
    assert!(elf.len() >= 0x40, "{ctx}: ELF shorter than ehdr");
    assert_eq!(&elf[..4], b"\x7fELF", "{ctx}: ELF magic");
    assert_eq!(elf[4], 2, "{ctx}: EI_CLASS must be ELFCLASS64");
    assert_eq!(elf[5], 2, "{ctx}: EI_DATA must be ELFDATA2MSB");
    let e_machine = u16::from_be_bytes([elf[0x12], elf[0x13]]);
    assert_eq!(e_machine, 21, "{ctx}: e_machine must be EM_PPC64 (21)");
    let e_phoff = u64::from_be_bytes(elf[0x20..0x28].try_into().unwrap()) as usize;
    let e_phentsize = usize::from(u16::from_be_bytes([elf[0x36], elf[0x37]]));
    let e_phnum = usize::from(u16::from_be_bytes([elf[0x38], elf[0x39]]));
    assert!(
        e_phnum > 0,
        "{ctx}: ELF must have at least one program header"
    );
    let phdr_table_end = e_phoff
        .checked_add(e_phnum.checked_mul(e_phentsize).unwrap())
        .unwrap();
    assert!(
        phdr_table_end <= elf.len(),
        "{ctx}: phdr table extends past ELF buffer: e_phoff=0x{e_phoff:x} + \
         e_phnum={e_phnum} * e_phentsize={e_phentsize} = {phdr_table_end} \
         > {} (ELF length)",
        elf.len(),
    );
}

#[test]
fn every_rap_derives_its_pinned_klicensee() {
    let vfs = keys::workspace_vfs();
    let keys = keys::vault();
    for v in load() {
        let rap_path = vfs.join(&v.rap);
        let rap = read_rap(&rap_path);
        let got = rap_to_klic(&keys, &rap)
            .unwrap_or_else(|e| panic!("{}: klicensee derivation failed: {e}", v.rap));
        assert_eq!(
            got,
            hex_to_bytes16(&v.klicensee, &v.rap),
            "{}: klicensee drift",
            v.rap
        );
    }
}

#[test]
fn every_pinned_eboot_decrypts_under_its_klicensee() {
    let vfs = keys::workspace_vfs();
    let keys = keys::vault();
    let manifest = load();
    let mut opened = 0usize;
    for v in &manifest {
        let Some(eboot) = &v.eboot else {
            continue;
        };
        let eboot_path = vfs.join(eboot);
        let bin = std::fs::read(&eboot_path)
            .unwrap_or_else(|e| panic!("read EBOOT {}: {e}", eboot_path.display()));
        let klic = hex_to_bytes16(&v.klicensee, &v.rap);
        let elf = decrypt_self_to_elf_npdrm(&bin, &keys, &klic).unwrap_or_else(|e| {
            panic!("{eboot}: NPDRM decrypt failed (padding + section hashes self-certify): {e}")
        });
        assert_is_ps3_ppc64_elf(&elf, eboot);
        opened += 1;
    }
    assert!(
        opened > 0,
        "{} pins no eboot, so no klicensee was proved against a container",
        manifest_path().display()
    );
}

/// The message [`parse`] refused `text` with.
fn refusal_message(text: &str) -> String {
    let payload = std::panic::catch_unwind(|| parse(text, "t"))
        .err()
        .unwrap_or_else(|| panic!("parse accepted {text:?}"));
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_else(|| panic!("the refusal of {text:?} carried a non-string payload"))
}

#[test]
fn a_manifest_with_no_vectors_is_rejected() {
    let msg = refusal_message("vector = []\n");
    assert!(msg.contains("pins no [[vector]]"), "got {msg:?}");
}

/// One well-formed row pinning `rap`, plus `extra` lines.
fn row(rap: &str, extra: &str) -> String {
    format!(
        "[[vector]]\nrap = \"{rap}\"\nklicensee = \"{}\"\n{extra}",
        "a".repeat(32)
    )
}

#[test]
fn an_unknown_field_is_named_rather_than_passed_over() {
    let msg = refusal_message(&row("x.rap", "ebot = \"e.bin\"\n"));
    assert!(msg.contains("unknown field"), "row level: got {msg:?}");
    let msg = refusal_message(&format!("vectors = []\n{}", row("x.rap", "")));
    assert!(msg.contains("unknown field"), "top level: got {msg:?}");
}

#[test]
fn a_repeated_rap_is_named_rather_than_derived_twice() {
    let text = format!("{}{}", row("x.rap", ""), row("x.rap", ""));
    let msg = refusal_message(&text);
    assert!(msg.contains("more than once"), "got {msg:?}");
}

#[test]
fn a_path_that_is_not_relative_under_the_vfs_is_rejected() {
    for bad in ["../x.rap", "/x.rap", "a/../../x.rap"] {
        let msg = refusal_message(&row(bad, ""));
        assert!(
            msg.contains("leaves the workspace"),
            "rap {bad:?}: got {msg:?}"
        );
        let msg = refusal_message(&row("x.rap", &format!("eboot = \"{bad}\"\n")));
        assert!(
            msg.contains("leaves the workspace"),
            "eboot {bad:?}: got {msg:?}"
        );
    }
    let msg = refusal_message(&row("", ""));
    assert!(msg.contains("is empty"), "got {msg:?}");
    let msg = refusal_message(&row("x.rap", "eboot = \"\"\n"));
    assert!(msg.contains("is empty"), "got {msg:?}");
}

#[test]
fn a_klicensee_that_is_not_32_hex_characters_is_rejected() {
    for bad in ["deadbeef", &"a".repeat(31), &"z".repeat(32)] {
        let text = format!("[[vector]]\nrap = \"x.rap\"\nklicensee = \"{bad}\"\n");
        let msg = refusal_message(&text);
        assert!(
            msg.contains("hex must be 32 chars") || msg.contains("invalid hex byte"),
            "klicensee {bad:?}: got {msg:?}"
        );
    }
}
