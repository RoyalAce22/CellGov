//! Building `firmware.toml` over a staged tree.

use super::*;
use crate::scratch_dir::scratch;

#[cfg(feature = "decrypt")]
fn digest(bytes: &[u8]) -> manifest::Sha256 {
    manifest::Sha256(manifest::sha256_of(bytes))
}

#[cfg(feature = "decrypt")]
fn build(
    dir: &std::path::Path,
) -> Result<(FirmwareManifest, Vec<ManifestOmission>), FirmwareInstallError> {
    build_manifest(
        digest(b"pup"),
        0,
        &crate::store::layout::VersionKey::new("4.91").unwrap(),
        dir,
        &crate::keys::KeyVault::empty(),
    )
}

#[test]
fn the_walk_collects_prx_and_sprx_from_every_depth_in_sorted_order() {
    let dir = scratch();
    std::fs::create_dir_all(dir.join("sys/external")).unwrap();
    std::fs::write(dir.join("sys/external/b.sprx"), b"B").unwrap();
    std::fs::write(dir.join("sys/external/a.PRX"), b"A").unwrap();
    std::fs::write(dir.join("sys/external/notes.txt"), b"N").unwrap();
    std::fs::write(dir.join("top.prx"), b"T").unwrap();

    let mut modules = Vec::new();
    collect_modules(&dir, "", &mut modules).expect("walk");
    let rel: Vec<&str> = modules.iter().map(|(_, r)| r.as_str()).collect();
    assert_eq!(
        rel,
        vec!["sys/external/a.PRX", "sys/external/b.sprx", "top.prx"]
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn an_unreadable_firmware_tree_is_named_rather_than_yielding_a_short_manifest() {
    let dir = scratch();
    assert!(matches!(
        build(&dir.join("absent")),
        Err(FirmwareInstallError::Io { op: "read dir", .. })
    ));

    let not_a_dir = dir.join("dev_flash.txt");
    std::fs::write(&not_a_dir, b"x").unwrap();
    assert!(matches!(
        build(&not_a_dir),
        Err(FirmwareInstallError::Io { op: "read dir", .. })
    ));
}

#[cfg(feature = "decrypt")]
#[test]
fn a_sprx_that_is_neither_an_sce_container_nor_an_elf_is_left_out_of_the_manifest() {
    let dir = scratch();
    std::fs::create_dir_all(dir.join("vsh/module")).unwrap();
    std::fs::write(dir.join("vsh/module/placeholder.sprx"), b"").unwrap();
    std::fs::write(dir.join("vsh/module/garbage.sprx"), b"not a module").unwrap();
    let mut bare_elf = ELF_MAGIC.to_vec();
    bare_elf.extend_from_slice(b"pre-decrypted body");
    std::fs::write(dir.join("vsh/module/plain.prx"), &bare_elf).unwrap();

    let (manifest, omissions) = build(&dir).expect("manifest");
    let paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["vsh/module/plain.prx"],
        "only the bare ELF is a module"
    );

    // The one entry is the ELF's own bytes, not the empty-bytes hash a
    // recorded placeholder would carry.
    assert_ne!(manifest.files[0].sha256, digest(b""));
    assert_eq!(manifest.files[0].sha256, digest(&bare_elf));

    let omitted: Vec<&str> = omissions
        .iter()
        .map(|o| match o {
            ManifestOmission::NotAModule { path, .. } => path.as_str(),
            ManifestOmission::Undecryptable { path, .. } => path.as_str(),
        })
        .collect();
    assert_eq!(
        omitted,
        vec!["vsh/module/garbage.sprx", "vsh/module/placeholder.sprx"],
        "both non-modules are named, not merely counted"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn an_sce_module_that_will_not_decrypt_is_omitted_carrying_the_decrypts_reason() {
    let dir = scratch();
    std::fs::create_dir_all(dir.join("sys/external")).unwrap();
    // A SELF-shaped container: magic, revision_flags 0x0018 at offset
    // 8, and a program identification header at 0xC0 typed APP -- the
    // decrypt reads the type before it consults the vault, and the
    // empty vault holds no APP key for that revision.
    let mut sce = vec![0u8; 0x100];
    sce[0..4].copy_from_slice(&cellgov_ps3_abi::format::sce::SCE_MAGIC);
    sce[9] = 0x18;
    sce[0x28..0x30].copy_from_slice(&0xC0u64.to_be_bytes());
    sce[0xCC..0xD0]
        .copy_from_slice(&cellgov_ps3_abi::format::sce::SELF_PROGRAM_TYPE_APP.to_be_bytes());
    std::fs::write(dir.join("sys/external/libsealed.sprx"), &sce).unwrap();

    let (manifest, omissions) = build(&dir).expect("manifest");
    assert!(
        manifest.files.is_empty(),
        "an unopened container carries no image to hash"
    );
    let [ManifestOmission::Undecryptable { path, reason }] = &omissions[..] else {
        panic!("expected one Undecryptable omission, got {omissions:?}");
    };
    assert_eq!(path, "sys/external/libsealed.sprx");
    assert!(
        reason.contains("0x0018"),
        "reason names no revision: {reason}"
    );
}

#[cfg(feature = "decrypt")]
#[test]
fn the_manifest_carries_the_version_the_entry_is_keyed_on() {
    let dir = scratch();
    let (manifest, _) = build(&dir).expect("manifest");
    assert_eq!(manifest.format_version, SUPPORTED_FORMAT_VERSION);
    assert_eq!(manifest.firmware.version, "4.91");
    assert_eq!(manifest.firmware.image_version, "0x0000000000000000");
    assert_eq!(manifest.firmware.pup_sha256, digest(b"pup"));
}
