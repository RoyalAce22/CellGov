//! ELF user-region sizing and boot-summary wire shape.

use super::*;
use crate::NullSink;

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
    assert_eq!(elf_user_region_end(&elf, &NullSink), 0x0082_0000 + 0x7_5CD4);
}

#[test]
fn elf_user_region_end_ignores_segments_above_user_range() {
    let elf = synthetic_elf(&[
        (0x0001_0000, 0x10_0000),
        (0x1000_0000, 0x4_0000),
        (0x1006_0000, 0x100),
    ]);
    assert_eq!(
        elf_user_region_end(&elf, &NullSink),
        0x0001_0000 + 0x10_0000
    );
}

#[test]
fn elf_user_region_end_skips_zero_memsz() {
    let elf = synthetic_elf(&[(0x0001_0000, 0), (0x0002_0000, 0x100)]);
    assert_eq!(elf_user_region_end(&elf, &NullSink), 0x0002_0000 + 0x100);
}

#[test]
fn elf_user_region_end_returns_zero_for_no_user_segments() {
    let elf = synthetic_elf(&[(0x1000_0000, 0x4_0000)]);
    assert_eq!(elf_user_region_end(&elf, &NullSink), 0);
}

#[test]
fn elf_user_region_end_rejects_non_elf_input() {
    assert_eq!(elf_user_region_end(&[0u8; 64], &NullSink), 0);
    assert_eq!(elf_user_region_end(&[0u8; 4], &NullSink), 0);
}

fn snapshots_with(spaces: &[(u32, cellgov_mem::GuestMemory)]) -> cellgov_compare::SpaceSnapshots {
    spaces
        .iter()
        .map(|(id, mem)| (cellgov_compare::AddressSpaceId::new(*id), mem.clone()))
        .collect()
}

fn child_result_region() -> Vec<cellgov_compare::RegionDescriptor> {
    vec![cellgov_compare::RegionDescriptor {
        name: "child_result".into(),
        space: cellgov_compare::AddressSpaceId::new(1),
        addr: 0x100,
        size: 4,
    }]
}

/// The caller must hold the returned guard while it uses the path.
fn temp_path(name: &str) -> (cellgov_testkit::scratch::ScratchDir, std::path::PathBuf) {
    let dir = cellgov_testkit::scratch::scratch_labeled(name);
    let path = dir.join("observation.json");
    (dir, path)
}

#[test]
fn a_region_naming_a_space_the_run_never_created_is_refused_not_zero_filled() {
    let (_dir, out) = temp_path("missing_space");
    let spaces = snapshots_with(&[(0, cellgov_mem::GuestMemory::new(0x1000))]);

    let err = save_boot_observation(ObservationInputs {
        path: out.to_str().unwrap(),
        elf_data: &[],
        final_spaces: &spaces,
        outcome: cellgov_compare::BootOutcome::ProcessExit,
        steps: 0,
        manifest_regions: Some(&child_result_region()),
        tty_log: &[],
        identity: &cellgov_compare::RunIdentity::default(),
        sink: &crate::NullSink,
    })
    .expect_err("space 1 was never created");
    match err {
        ObservationSaveError::Region(cellgov_compare::RegionExtractError::SpaceMissing {
            name,
            space,
            present,
        }) => {
            assert_eq!(name, "child_result");
            assert_eq!(space, 1);
            assert_eq!(present, vec![0]);
        }
        other => panic!("expected Region(SpaceMissing), got {other:?}"),
    }
    assert!(
        !out.exists(),
        "a refused manifest must not leave a zero-filled observation behind"
    );
}

#[test]
fn a_region_in_a_created_child_space_captures_that_space() {
    let (_dir, out) = temp_path("child_space");
    let mut child = cellgov_mem::GuestMemory::new(0x1000);
    let range =
        cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0x100), 4).expect("4-byte range");
    child
        .apply_commit(range, &[0xDE, 0xAD, 0xBE, 0xEF])
        .expect("range inside the child region");
    let spaces = snapshots_with(&[(0, cellgov_mem::GuestMemory::new(0x1000)), (1, child)]);

    save_boot_observation(ObservationInputs {
        path: out.to_str().unwrap(),
        elf_data: &[],
        final_spaces: &spaces,
        outcome: cellgov_compare::BootOutcome::ProcessExit,
        steps: 0,
        manifest_regions: Some(&child_result_region()),
        tty_log: &[],
        identity: &cellgov_compare::RunIdentity::default(),
        sink: &crate::NullSink,
    })
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
}
