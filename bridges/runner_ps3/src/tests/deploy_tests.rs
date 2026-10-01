//! Deployment: the package's paths from the manifest, the order and
//! contents of what reaches the console, and a missing build output
//! that changes nothing on it.

use cellgov_compare::manifest;
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
