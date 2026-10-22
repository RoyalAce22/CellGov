//! NPDRM witness vectors over the operator's installed content.
//!
//! The operator pins, in `vfs/.cellgov/npdrm_oracle_vectors.toml`,
//! RAP files under the workspace `vfs/` with the klicensee each must
//! derive to, and the NPDRM EBOOT that klicensee opens. The schema is
//! in `common/npdrm_manifest.rs`.
//!
//! Compiled only under `npdrm-oracle-vectors`, which declares the
//! manifest present: a missing or empty manifest fails, as does one
//! that pins no EBOOT.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: unwrap on unexpected failure is correct"
)]

use std::path::{Path, PathBuf};

use cellgov_install::npdrm::{decrypt_self_to_elf_npdrm, rap_to_klic};

#[path = "common/keys.rs"]
mod keys;
#[path = "common/npdrm_manifest.rs"]
mod npdrm_manifest;
use npdrm_manifest::{hex_to_bytes16, parse, Vector};

/// Manifest location, relative to the workspace `vfs/`.
const MANIFEST: &str = ".cellgov/npdrm_oracle_vectors.toml";

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
