//! The default profiles file resolves against the workspace root, not
//! the working directory: `runner_ps3 convert` without `--profiles`
//! loads the tracked file when it runs from an empty scratch directory.

use std::path::Path;
use std::process::Command;

#[test]
fn the_default_profiles_file_is_found_from_any_working_directory() {
    let test = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/micro/spu_fixed_value");
    let scratch = cellgov_testkit::scratch::scratch();
    let output = Command::new(env!("CARGO_BIN_EXE_runner_ps3"))
        .current_dir(&scratch)
        .args(["convert", "--profile", "cech20-cex-493", "--frame"])
        .arg(test.join("ps3/cech20-cex-493/cgov_frame.bin"))
        .arg("--manifest")
        .arg(test.join("manifest.toml"))
        .output()
        .expect("runner_ps3 runs");
    assert!(
        output.status.success(),
        "exit {:?}: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        output.stdout.starts_with(b"{"),
        "the observation is the report: {}",
        String::from_utf8_lossy(&output.stdout)
    );
}
