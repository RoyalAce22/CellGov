use super::*;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::loader_images::seeds;

/// Cases each target runs in the bounded sweep; small, so the
/// continuous build stays short. `LOADER_SWEEP_CASES` raises it for a
/// soak run.
const SWEEP_CASES: u64 = 400;

fn sweep_cases() -> u64 {
    std::env::var("LOADER_SWEEP_CASES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(SWEEP_CASES)
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("crate manifest dir is two levels under the workspace root")
        .to_path_buf()
}

fn seed_bytes() -> Vec<Vec<u8>> {
    seeds().into_iter().map(|seed| seed.bytes).collect()
}

fn seed(name: &str) -> Vec<u8> {
    seeds()
        .into_iter()
        .find(|seed| seed.name == name)
        .unwrap_or_else(|| panic!("no seed named {name}"))
        .bytes
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[test]
fn target_names_round_trip_and_are_distinct() {
    let mut names: Vec<&str> = LoaderTarget::ALL.iter().map(|t| t.name()).collect();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), LoaderTarget::ALL.len());
    for target in LoaderTarget::ALL {
        assert_eq!(LoaderTarget::parse_name(target.name()), Some(target));
        assert_eq!(target.to_string(), target.name());
    }
    assert_eq!(LoaderTarget::parse_name("decode"), None);
}

#[test]
fn every_target_accepts_the_seed_built_for_it_and_refuses_the_wrong_shape() {
    let accepted = [
        (LoaderTarget::PtLoadSegments, "exec_text_only"),
        (LoaderTarget::LoadPpuElf, "exec_entry_opd"),
        (LoaderTarget::ParsePrx, "prx_baseline"),
        (LoaderTarget::ParseImports, "prx_imports"),
        (LoaderTarget::FuncmapBuild, "exec_entry_opd"),
    ];
    for (target, name) in accepted {
        assert_eq!(
            run(target, &seed(name)),
            LoaderOutcome::Accepted,
            "{target} on {name}"
        );
    }
    assert_eq!(
        run(LoaderTarget::ParsePrx, &seed("exec_text_only")),
        LoaderOutcome::Refused(LoaderRefusal::Prx(PrxParseError::NotPrx(2)))
    );
    assert_eq!(
        run(LoaderTarget::PtLoadSegments, b"not an elf"),
        LoaderOutcome::Refused(LoaderRefusal::Elf(LoadError::TooSmall))
    );
}

#[test]
fn exercise_runs_the_raw_bytes_and_the_image_they_describe() {
    let outcome = exercise(LoaderTarget::PtLoadSegments, &[]);
    assert_eq!(
        outcome,
        CaseOutcome {
            raw: LoaderOutcome::Refused(LoaderRefusal::Elf(LoadError::TooSmall)),
            // An empty stream describes an image with no program
            // headers, which the reader refuses by name.
            structured: LoaderOutcome::Refused(LoaderRefusal::Elf(LoadError::NoProgramHeaders)),
        }
    );
}

#[test]
fn a_bounded_sweep_reaches_both_outcomes_and_panics_nowhere() {
    let seeds = seed_bytes();
    let cases = sweep_cases();
    for target in LoaderTarget::ALL {
        let report = sweep(
            target,
            &seeds,
            SweepConfig {
                seed: 1,
                cases,
                max_panics: 4,
            },
        )
        .unwrap();
        assert_eq!(report.cases, cases);
        let replay: Vec<String> = report
            .panics
            .iter()
            .map(|p| format!("{:?} {:?}: {}", p.path, p.payload, hex(&p.input)))
            .collect();
        assert_eq!(
            report.panicked, 0,
            "{target}: {} panic(s); the first: {replay:#?}",
            report.panicked
        );
        assert!(report.accepted > 0, "{target} accepted nothing");
        assert!(report.refused > 0, "{target} refused nothing");
    }
}

#[test]
fn a_sweep_replays_the_same_counts_for_the_same_seed() {
    let seeds = seed_bytes();
    let config = SweepConfig {
        seed: 7,
        cases: 50,
        max_panics: 1,
    };
    let first = sweep(LoaderTarget::ParsePrx, &seeds, config).unwrap();
    let second = sweep(LoaderTarget::ParsePrx, &seeds, config).unwrap();
    assert_eq!(first, second);
    // Another seed draws other mutations from the same base.
    let base = &seeds[0];
    let mut seven = Rng::for_case(CAMPAIGN_VERSION, 7, 0);
    let mut eight = Rng::for_case(CAMPAIGN_VERSION, 8, 0);
    assert_ne!(
        mutate(&mut seven, base).unwrap(),
        mutate(&mut eight, base).unwrap()
    );
}

#[test]
fn a_sweep_over_no_seeds_is_a_generator_error() {
    let result = sweep(
        LoaderTarget::ParsePrx,
        &[],
        SweepConfig {
            seed: 1,
            cases: 1,
            max_panics: 1,
        },
    );
    assert_eq!(result, Err(GeneratorError::ZeroProbabilityDenominator));
}

#[test]
fn mutation_changes_the_input_within_its_bounds() {
    let base = seed("prx_baseline");
    let mut changed = 0;
    for case in 0..32 {
        let mut rng = Rng::for_case(CAMPAIGN_VERSION, 3, case);
        let out = mutate(&mut rng, &base).unwrap();
        assert!(out.len() <= MAX_INPUT_BYTES);
        if out != base {
            changed += 1;
        }
    }
    assert!(
        changed >= 24,
        "only {changed} of 32 mutations changed the input"
    );
    let mut rng = Rng::for_case(CAMPAIGN_VERSION, 3, 0);
    assert!(!mutate(&mut rng, &[]).unwrap().is_empty());
}

fn seed_dir() -> PathBuf {
    workspace_root().join("fuzz").join("seeds")
}

fn committed_seeds() -> BTreeMap<String, Vec<u8>> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(seed_dir()).expect("fuzz/seeds exists") {
        let path = entry.expect("seed entry").path();
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        if path.extension().and_then(|e| e.to_str()) == Some("bin") {
            out.insert(
                stem.to_owned(),
                std::fs::read(&path).expect("seed readable"),
            );
        }
    }
    out
}

fn generated_seeds() -> BTreeMap<String, Vec<u8>> {
    seeds()
        .into_iter()
        .map(|seed| (seed.name.to_owned(), seed.bytes))
        .collect()
}

#[test]
fn the_committed_seeds_match_the_generator() {
    let committed = committed_seeds();
    let generated = generated_seeds();
    let committed_names: Vec<&String> = committed.keys().collect();
    let generated_names: Vec<&String> = generated.keys().collect();
    assert_eq!(
        committed_names, generated_names,
        "fuzz/seeds holds a different set; run the ignored regenerate_seeds test"
    );
    for (name, bytes) in &generated {
        assert_eq!(
            &committed[name], bytes,
            "fuzz/seeds/{name}.bin differs; run the ignored regenerate_seeds test"
        );
    }
}

/// `cargo test -p cellgov_fuzz --lib -- --ignored regenerate_seeds`
#[test]
#[ignore = "rewrites fuzz/seeds; run it after changing the seed images"]
fn regenerate_seeds() {
    let dir = seed_dir();
    std::fs::create_dir_all(&dir).expect("create fuzz/seeds");
    for (name, _) in committed_seeds() {
        std::fs::remove_file(dir.join(format!("{name}.bin"))).expect("remove stale seed");
    }
    for (name, bytes) in generated_seeds() {
        std::fs::write(dir.join(format!("{name}.bin")), bytes).expect("write seed");
    }
}

#[test]
fn the_fuzz_package_and_the_workflow_name_exactly_the_targets() {
    let root = workspace_root();
    let expected: Vec<&str> = LoaderTarget::ALL.iter().map(|t| t.name()).collect();

    let manifest =
        std::fs::read_to_string(root.join("fuzz").join("Cargo.toml")).expect("fuzz/Cargo.toml");
    let bins: Vec<&str> = manifest
        .lines()
        .filter_map(|line| line.trim().strip_prefix("name = \""))
        .filter_map(|rest| rest.strip_suffix('"'))
        .filter(|name| !name.contains('-'))
        .collect();
    assert_eq!(bins, expected, "fuzz/Cargo.toml [[bin]] names");
    for name in &expected {
        assert!(
            manifest.contains(&format!("path = \"fuzz_targets/{name}.rs\"")),
            "fuzz/Cargo.toml names no path for {name}"
        );
    }

    let mut files: Vec<String> = std::fs::read_dir(root.join("fuzz").join("fuzz_targets"))
        .expect("fuzz/fuzz_targets")
        .map(|entry| entry.expect("target entry").path())
        .filter_map(|path| path.file_stem()?.to_str().map(str::to_owned))
        .collect();
    files.sort();
    let mut sorted = expected.clone();
    sorted.sort_unstable();
    assert_eq!(files, sorted, "fuzz/fuzz_targets files");
    for name in &expected {
        let source = std::fs::read_to_string(
            root.join("fuzz")
                .join("fuzz_targets")
                .join(format!("{name}.rs")),
        )
        .expect("target source");
        let variant = LoaderTarget::parse_name(name).expect("named target");
        assert!(
            source.contains(&format!("LoaderTarget::{variant:?}")),
            "{name}.rs does not call its own target"
        );
    }

    let workflow = std::fs::read_to_string(root.join(".github/workflows/loader-fuzz.yml"))
        .expect("loader-fuzz workflow");
    let matrix = format!("target: [{}]", expected.join(", "));
    assert!(
        workflow.contains(&matrix),
        "loader-fuzz.yml matrix is not `{matrix}`"
    );
}
