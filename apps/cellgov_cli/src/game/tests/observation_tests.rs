//! ELF user-region sizing, checkpoint-manifest parsing, and boot-summary wire shape.

use super::*;

fn synthetic_elf(loads: &[(u64, u64)]) -> Vec<u8> {
    let phoff: u64 = 64;
    let phentsize: u16 = 56;
    let phnum: u16 = loads.len() as u16;
    let header_end = phoff as usize + phentsize as usize * phnum as usize;
    let mut buf = vec![0u8; header_end];
    buf[0..4].copy_from_slice(&[0x7f, b'E', b'L', b'F']);
    buf[4] = 2; // ELFCLASS64
    buf[5] = 2; // ELFDATA2MSB (big-endian)
    buf[32..40].copy_from_slice(&phoff.to_be_bytes());
    buf[54..56].copy_from_slice(&phentsize.to_be_bytes());
    buf[56..58].copy_from_slice(&phnum.to_be_bytes());
    for (i, &(vaddr, memsz)) in loads.iter().enumerate() {
        let base = phoff as usize + i * phentsize as usize;
        buf[base..base + 4].copy_from_slice(&1u32.to_be_bytes()); // PT_LOAD
        buf[base + 16..base + 24].copy_from_slice(&vaddr.to_be_bytes());
        buf[base + 40..base + 48].copy_from_slice(&memsz.to_be_bytes());
    }
    buf
}

#[test]
fn elf_user_region_end_picks_max_in_user_range() {
    let elf = synthetic_elf(&[(0x0001_0000, 0x80_0000), (0x0082_0000, 0x7_5CD4)]);
    assert_eq!(elf_user_region_end(&elf), 0x0082_0000 + 0x7_5CD4);
}

#[test]
fn elf_user_region_end_ignores_segments_above_user_range() {
    let elf = synthetic_elf(&[
        (0x0001_0000, 0x10_0000),
        (0x1000_0000, 0x4_0000),
        (0x1006_0000, 0x100),
    ]);
    assert_eq!(elf_user_region_end(&elf), 0x0001_0000 + 0x10_0000);
}

#[test]
fn elf_user_region_end_skips_zero_memsz() {
    let elf = synthetic_elf(&[(0x0001_0000, 0), (0x0002_0000, 0x100)]);
    assert_eq!(elf_user_region_end(&elf), 0x0002_0000 + 0x100);
}

#[test]
fn elf_user_region_end_returns_zero_for_no_user_segments() {
    let elf = synthetic_elf(&[(0x1000_0000, 0x4_0000)]);
    assert_eq!(elf_user_region_end(&elf), 0);
}

#[test]
fn elf_user_region_end_rejects_non_elf_input() {
    assert_eq!(elf_user_region_end(&[0u8; 64]), 0);
    assert_eq!(elf_user_region_end(&[0u8; 4]), 0);
}

fn parse(text: &str) -> CheckpointManifest {
    toml::from_str(text).expect("parses")
}

#[test]
fn checkpoint_manifest_parses_hex_addresses() {
    let m = parse(
        r#"
        [[regions]]
        name = "code"
        addr = "0x10000"
        size = "0x800000"

        [[regions]]
        name = "rodata"
        addr = "0x10000000"
        size = "0x40000"
        "#,
    );
    assert_eq!(m.regions.len(), 2);
    let CheckpointRegion {
        ref name,
        space,
        addr,
        size,
    } = m.regions[0];
    assert_eq!(name, "code");
    assert_eq!(space, 0, "an absent space field means the boot space");
    assert_eq!(addr, 0x10000);
    assert_eq!(size, 0x800000);
    assert_eq!(m.regions[1].addr, 0x1000_0000);
    assert_eq!(m.regions[1].size, 0x40000);
}

#[test]
fn checkpoint_manifest_carries_a_child_space() {
    let m = parse(
        r#"
        [[regions]]
        name = "child_result"
        space = 1
        addr = "0x100"
        size = "0x10"
        "#,
    );
    assert_eq!(m.regions[0].space, 1);
}

#[test]
fn checkpoint_manifest_rejects_a_negative_space() {
    let bad = toml::from_str::<CheckpointManifest>(
        r#"
        [[regions]]
        name = "r"
        space = -1
        addr = "0x100"
        size = "0x10"
        "#,
    );
    assert!(
        bad.is_err(),
        "a negative space cannot name any address space and must not wrap"
    );
}

fn temp_path(name: &str, ext: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "cellgov_observation_{name}_{}.{ext}",
        std::process::id()
    ))
}

fn snapshots_with(spaces: &[(u32, cellgov_mem::GuestMemory)]) -> cellgov_compare::SpaceSnapshots {
    spaces
        .iter()
        .map(|(id, mem)| (cellgov_compare::AddressSpaceId::new(*id), mem.clone()))
        .collect()
}

const CHILD_REGION_MANIFEST: &str = r#"
[[regions]]
name = "child_result"
space = 1
addr = "0x100"
size = "0x4"
"#;

#[test]
fn a_region_naming_a_space_the_run_never_created_is_refused_not_zero_filled() {
    let manifest = temp_path("missing_space", "toml");
    std::fs::write(&manifest, CHILD_REGION_MANIFEST).expect("write manifest");
    let out = temp_path("missing_space", "json");
    let spaces = snapshots_with(&[(0, cellgov_mem::GuestMemory::new(0x1000))]);

    let err = save_boot_observation(
        out.to_str().unwrap(),
        &[],
        &spaces,
        cellgov_compare::BootOutcome::ProcessExit,
        0,
        Some(manifest.to_str().unwrap()),
        &[],
    )
    .expect_err("space 1 was never created");
    match err {
        ObservationSaveError::RegionSpaceMissing {
            region,
            space,
            present,
        } => {
            assert_eq!(region, "child_result");
            assert_eq!(space, 1);
            assert_eq!(present, vec![0]);
        }
        other => panic!("expected RegionSpaceMissing, got {other:?}"),
    }
    assert!(
        !out.exists(),
        "a refused manifest must not leave a zero-filled observation behind"
    );
    std::fs::remove_file(&manifest).expect("remove temp manifest");
}

#[test]
fn a_region_in_a_created_child_space_captures_that_space() {
    let manifest = temp_path("child_space", "toml");
    std::fs::write(&manifest, CHILD_REGION_MANIFEST).expect("write manifest");
    let out = temp_path("child_space", "json");
    let mut child = cellgov_mem::GuestMemory::new(0x1000);
    let range =
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x100), 4).expect("4-byte range");
    child
        .apply_commit(range, &[0xDE, 0xAD, 0xBE, 0xEF])
        .expect("range inside the child region");
    let spaces = snapshots_with(&[(0, cellgov_mem::GuestMemory::new(0x1000)), (1, child)]);

    save_boot_observation(
        out.to_str().unwrap(),
        &[],
        &spaces,
        cellgov_compare::BootOutcome::ProcessExit,
        0,
        Some(manifest.to_str().unwrap()),
        &[],
    )
    .expect("space 1 exists");
    let text = std::fs::read_to_string(&out).expect("read observation");
    let obs: cellgov_compare::Observation = serde_json::from_str(&text).expect("parses");
    assert_eq!(obs.memory_regions.len(), 1);
    assert_eq!(obs.memory_regions[0].name, "child_result");
    assert_eq!(
        obs.memory_regions[0].data,
        vec![0xDE, 0xAD, 0xBE, 0xEF],
        "the bytes come from space 1, whose boot-space twin at the same address is zero"
    );
    std::fs::remove_file(&manifest).expect("remove temp manifest");
    std::fs::remove_file(&out).expect("remove temp observation");
}

#[test]
fn checkpoint_manifest_accepts_unprefixed_hex() {
    let m = parse(
        r#"
        [[regions]]
        name = "r"
        addr = "1000"
        size = "10"
        "#,
    );
    assert_eq!(m.regions[0].addr, 0x1000);
    assert_eq!(m.regions[0].size, 0x10);
}

#[test]
fn checkpoint_manifest_rejects_non_hex_value() {
    let bad = toml::from_str::<CheckpointManifest>(
        r#"
        [[regions]]
        name = "r"
        addr = "not-hex"
        size = "10"
        "#,
    );
    assert!(bad.is_err(), "non-hex addr must fail");
}

#[test]
fn checkpoint_manifest_loads_committed_fixture() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fixtures")
        .join("NPUA80001")
        .join("checkpoint.toml");
    let text = std::fs::read_to_string(&path).expect("read");
    let m: CheckpointManifest = toml::from_str(&text).expect("parses");
    assert!(!m.regions.is_empty());
    assert!(m.regions.iter().any(|r| r.name == "code"));
}

mod boot_summary_cross_check {
    //! Pin the JSON wire shape `cellgov_cli`'s `CheckpointTrigger`
    //! produces against `cellgov_compare::CheckpointKind`. When a
    //! new variant lands on either side, this test fails first.

    use super::checkpoint_to_kind;
    use cellgov_mem::GuestAddr;

    #[test]
    fn each_trigger_maps_to_matching_kind_json() {
        let cli = crate::game::manifest::CheckpointTrigger::ProcessExit;
        assert_eq!(
            serde_json::to_value(checkpoint_to_kind(cli)).unwrap(),
            serde_json::json!({ "kind": "process_exit" }),
        );

        let cli = crate::game::manifest::CheckpointTrigger::FirstRsxWrite;
        assert_eq!(
            serde_json::to_value(checkpoint_to_kind(cli)).unwrap(),
            serde_json::json!({ "kind": "first_rsx_write" }),
        );

        let cli = crate::game::manifest::CheckpointTrigger::Pc(0x10381ce8);
        assert_eq!(
            serde_json::to_value(checkpoint_to_kind(cli)).unwrap(),
            serde_json::json!({ "kind": "pc", "addr": GuestAddr::new(0x10381ce8).raw() }),
        );
    }
}
