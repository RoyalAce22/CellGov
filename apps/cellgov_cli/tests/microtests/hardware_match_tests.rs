//! CellGov's payload against the console's: for each bootable microtest
//! with a capture under the reference profile, the bytes CellGov emits
//! equal the bytes the console emitted, except where the manifest
//! declares them volatile. A microtest with a known CellGov defect must
//! still differ.

use cellgov_compare::console_profile::{ConsoleProfiles, CONSOLE_PROFILES_FILE};
use cellgov_compare::hardware_capture::{self, CAPTURE_DIR};
use cellgov_compare::manifest;
use cellgov_compare::observation::blank_volatile;
use cellgov_compare::runner_rpcs3::{parse_tty_frame, TtyRegion};
use cellgov_compare::{compare, Classification, CompareMode};

use super::*;

#[test]
fn every_microtest_matches_its_hardware_capture() {
    let micro = workspace_root().join("tests").join("micro");
    let profiles = ConsoleProfiles::load(&micro.join(CONSOLE_PROFILES_FILE))
        .expect("the tracked profiles load");
    let mut problems = Vec::new();
    for case in CASES {
        let capture = micro
            .join(case.name)
            .join(CAPTURE_DIR)
            .join(&profiles.reference);
        if !capture.is_dir() {
            continue;
        }
        let m = manifest::load_console(&manifest_path(case.name)).expect("manifest");
        let mut console = hardware_capture::load(&capture, &profiles)
            .unwrap_or_else(|e| panic!("{}: {e}", capture.display()))
            .observation;
        let regions: Vec<TtyRegion> = m
            .observe
            .memory_regions
            .iter()
            .map(|r| TtyRegion {
                name: r.name.clone(),
                offset: r.payload_offset(),
                size: r.size,
                guest_addr: r.addr,
            })
            .collect();
        let booted = run_observation(case, "hardware");
        let difference = match check_outcome(case, &booted) {
            Some(problem) => Some(problem),
            None => match parse_tty_frame(&booted.tty_log, &regions) {
                Err(e) => Some(e.to_string()),
                Ok(ours) => {
                    // Only the regions are compared: the console's
                    // observation is the frame alone, with no outcome or
                    // events to hold against a boot's.
                    let mut cellgov = console.clone();
                    cellgov.memory_regions = ours;
                    blank_volatile(&mut console, &m.ps3.volatile);
                    blank_volatile(&mut cellgov, &m.ps3.volatile);
                    let result = compare(&console, &cellgov, CompareMode::Memory);
                    (result.classification != Classification::Match).then(|| format!("{result:?}"))
                }
            },
        };
        match (difference, known_defect(case.name)) {
            (Some(problem), None) => problems.push(format!("{}: {problem}", case.name)),
            (None, Some(issue)) => problems.push(format!(
                "{}: matches its capture, so #{issue} no longer reproduces; drop its \
                 KNOWN_DEFECTS entry",
                case.name
            )),
            _ => {}
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
