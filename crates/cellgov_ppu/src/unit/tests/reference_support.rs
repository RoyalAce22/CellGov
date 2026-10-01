//! The reference a microtest's CellGov run is held to: its console
//! capture under the reference profile when one is committed, the two
//! emulator baselines otherwise.

use std::path::Path;

use cellgov_compare::console_profile::{ConsoleProfiles, CONSOLE_PROFILES_FILE};
use cellgov_compare::hardware_capture::{microtest_reference, MicrotestReference};
use cellgov_compare::observation::blank_volatile;
use cellgov_compare::{baseline, compare, compare_multi, manifest, Classification, CompareMode};
use cellgov_compare::{CompareResult, Observation};

const MICRO: &str = "../../tests/micro";
const SCENARIOS: &str = "../../tests/scenario_observations";

/// Assert `cellgov` matches microtest `name`'s reference in memory. A
/// console capture is compared with both sides' volatile bytes blanked;
/// without one, the run must match both emulator baselines. The failure
/// names the reference it was held to.
pub(crate) fn assert_matches_reference(name: &str, cellgov: &Observation) {
    let micro = Path::new(MICRO);
    let profiles = ConsoleProfiles::load(&micro.join(CONSOLE_PROFILES_FILE))
        .expect("the tracked profiles load");
    let dir = micro.join(name);
    let reference = microtest_reference(&dir, &profiles)
        .unwrap_or_else(|e| panic!("{name}: the console capture does not load: {e}"));
    match reference {
        MicrotestReference::Hardware(capture) => {
            let m = manifest::load_console(&dir.join("manifest.toml"))
                .unwrap_or_else(|e| panic!("{name}: {e}"));
            let mut console = capture.observation;
            let mut ours = cellgov.clone();
            blank_volatile(&mut console, &m.ps3.volatile);
            blank_volatile(&mut ours, &m.ps3.volatile);
            let result: CompareResult = compare(&console, &ours, CompareMode::Memory);
            assert_eq!(
                result.classification,
                Classification::Match,
                "{name} diverges from its reference, the console capture under {}: {result:?}",
                profiles.reference
            );
        }
        MicrotestReference::EmulatorOnly => {
            let baselines = Path::new(SCENARIOS).join(name);
            let observations = ["rpcs3_interpreter.json", "rpcs3_llvm.json"].map(|file| {
                baseline::load(&baselines.join(file))
                    .unwrap_or_else(|e| panic!("{name}: {file}: {e}"))
            });
            let result = compare_multi(&observations, cellgov, CompareMode::Memory);
            assert_eq!(
                result.classification,
                Classification::Match,
                "{name} diverges from its reference, the two emulator baselines (no console capture yet): {:?}",
                result.cellgov_result
            );
        }
    }
}
