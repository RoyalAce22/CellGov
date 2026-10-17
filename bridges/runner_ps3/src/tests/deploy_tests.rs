//! Deployment: the package's paths from the manifest, the order and
//! contents of what reaches the console, and a missing build output
//! that changes nothing on it.

use cellgov_observation::manifest;
use cellgov_testkit::scratch::ScratchDir;

use super::*;
use crate::memory_console::{package_on_disk, MemoryConsole};

fn packaged() -> (ScratchDir, Package, Target) {
    let scratch = cellgov_testkit::scratch::scratch();
    let manifest_path = package_on_disk(&scratch);
    let manifest = manifest::load_console(&manifest_path).expect("manifest");
    let target = Target::new(&manifest.ps3.appid, &manifest.result_file_name());
    (scratch, Package::of(&manifest_path, &manifest), target)
}

#[test]
fn the_package_sits_under_build_ps3_with_the_reference_beside_it() {
    let (_scratch, package, _) = packaged();
    let ps3 = package.test_dir.join("build").join("ps3");
    assert_eq!(package.eboot, ps3.join("EBOOT.BIN"));
    assert_eq!(package.param_sfo, ps3.join("PARAM.SFO"));
    assert_eq!(package.ps3_elf, ps3.join("spu_fixed_value.elf"));
    assert_eq!(
        package.reference_elf,
        package.test_dir.join("build").join("spu_fixed_value.elf")
    );
    assert_eq!(
        package.siblings,
        [("spu_main.elf".to_string(), ps3.join("spu_main.elf"))]
    );
}

#[test]
fn deploy_creates_the_game_directory_and_stores_every_file_in_place() {
    let (_scratch, package, target) = packaged();
    let mut console = MemoryConsole::empty();
    deploy(&mut console, &target, &package, &mut Transcript::new()).expect("deployed");
    assert_eq!(
        console.calls,
        [
            "MKD /dev_hdd0/game/CGOV00001",
            "MKD /dev_hdd0/game/CGOV00001/USRDIR",
            "STOR /dev_hdd0/game/CGOV00001/PARAM.SFO (9 bytes)",
            "STOR /dev_hdd0/game/CGOV00001/USRDIR/EBOOT.BIN (9 bytes)",
            "STOR /dev_hdd0/game/CGOV00001/USRDIR/spu_main.elf (12 bytes)",
        ]
    );
    assert_eq!(
        console.files["/dev_hdd0/game/CGOV00001/USRDIR/spu_main.elf"],
        b"spu_main.elf"
    );
}

#[test]
fn a_relink_that_sends_a_passive_only_call_is_refused_before_the_console_is_touched() {
    let (_scratch, package, target) = packaged();
    let send = 0x3960_0000 | cellgov_ps3_abi::lv2::syscall::UART_SEND as u32;
    std::fs::write(
        &package.ps3_elf,
        crate::memory_console::ppu_elf(&[send, 0x4400_0002]),
    )
    .expect("relink");
    let mut console = MemoryConsole::empty();
    match deploy(&mut console, &target, &package, &mut Transcript::new()) {
        Err(RunnerPs3Error::PassiveOnlySyscall { elf, number, .. }) => {
            assert_eq!(elf, package.ps3_elf);
            assert_eq!(number, cellgov_ps3_abi::lv2::syscall::UART_SEND);
        }
        other => panic!("{other:?}"),
    }
    assert!(console.calls.is_empty(), "{:?}", console.calls);
}

#[test]
fn a_missing_build_output_is_named_and_nothing_reaches_the_console() {
    let (_scratch, package, target) = packaged();
    std::fs::remove_file(&package.siblings[0].1).expect("remove");
    let mut console = MemoryConsole::empty();
    match deploy(&mut console, &target, &package, &mut Transcript::new()) {
        Err(RunnerPs3Error::LocalRead { path, .. }) => assert_eq!(path, package.siblings[0].1),
        other => panic!("{other:?}"),
    }
    assert!(console.calls.is_empty(), "{:?}", console.calls);
}

/// `li r11, sys_uart_send` then `sc`.
const SENDS_UART: [u32; 2] = [
    0x3960_0000 | cellgov_ps3_abi::lv2::syscall::UART_SEND as u32,
    0x4400_0002,
];

/// `packaged()` with one more sibling, `name`, holding `bytes`.
fn with_sibling(name: &str, bytes: &[u8]) -> (ScratchDir, Package, Target) {
    let (scratch, mut package, target) = packaged();
    let path = package.ps3_elf.with_file_name(name);
    std::fs::write(&path, bytes).expect("sibling");
    package.siblings.push((name.to_string(), path));
    (scratch, package, target)
}

fn refused_number(package: &Package, target: &Target) -> (std::path::PathBuf, u64) {
    let mut console = MemoryConsole::empty();
    let result = deploy(&mut console, target, package, &mut Transcript::new());
    assert!(console.calls.is_empty(), "{:?}", console.calls);
    match result {
        Err(RunnerPs3Error::PassiveOnlySyscall { elf, number, .. }) => (elf, number),
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_ppu_program_sibling_that_sends_a_passive_only_call_is_refused() {
    let (_scratch, package, target) =
        with_sibling("child.elf", &crate::memory_console::ppu_elf(&SENDS_UART));
    assert_eq!(
        refused_number(&package, &target),
        (
            package.siblings[1].1.clone(),
            cellgov_ps3_abi::lv2::syscall::UART_SEND
        )
    );
}

#[test]
fn a_self_sibling_is_scanned_through_the_plain_elf_beside_it() {
    let (_scratch, package, target) = with_sibling("child.self", b"SCE\0wrapped");
    let elf = package.siblings[1].1.with_extension("elf");
    std::fs::write(&elf, crate::memory_console::ppu_elf(&SENDS_UART)).expect("plain elf");
    assert_eq!(
        refused_number(&package, &target),
        (elf, cellgov_ps3_abi::lv2::syscall::UART_SEND)
    );
}

#[test]
fn a_self_sibling_with_no_plain_elf_beside_it_is_refused() {
    let (_scratch, package, target) = with_sibling("child.self", b"SCE\0wrapped");
    let mut console = MemoryConsole::empty();
    match deploy(&mut console, &target, &package, &mut Transcript::new()) {
        Err(RunnerPs3Error::SelfWithoutElf { sibling, elf }) => {
            assert_eq!(sibling, package.siblings[1].1);
            assert_eq!(elf, sibling.with_extension("elf"));
            let message = RunnerPs3Error::SelfWithoutElf { sibling, elf }.to_string();
            assert!(
                message.ends_with("cannot be scanned; rebuild the probe"),
                "{message}"
            );
        }
        other => panic!("{other:?}"),
    }
    assert!(console.calls.is_empty(), "{:?}", console.calls);
}

/// An SPU image sends no syscall, so an ELF32 sibling is not scanned even
/// when a word of it equals a passive-only number.
#[test]
fn an_spu_image_sibling_is_not_scanned() {
    let mut spu = b"\x7fELF\x01\x02\x01\0".to_vec();
    spu.extend_from_slice(&(cellgov_ps3_abi::lv2::syscall::UART_SEND as u32).to_be_bytes());
    let (_scratch, package, target) = with_sibling("other_spu.elf", &spu);
    let mut console = MemoryConsole::empty();
    deploy(&mut console, &target, &package, &mut Transcript::new()).expect("deployed");
    assert!(console
        .files
        .contains_key("/dev_hdd0/game/CGOV00001/USRDIR/other_spu.elf"));
}
