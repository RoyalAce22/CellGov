//! Test cases for the microtest boot harness in `microtests.rs`.

use super::*;

#[test]
fn every_microtest_reports_its_documented_payload() {
    let mut failures: Vec<String> = Vec::new();
    for case in CASES {
        let observation = run_observation(case, "payload");
        if let Some(bad) = check_outcome(case, &observation) {
            failures.push(format!("{}: {bad}", case.name));
            continue;
        }
        let problems = check_payload(case, &observation);
        if problems.is_empty() {
            eprintln!(
                "microtest {}: PASS (steps={:?})",
                case.name, observation.metadata.steps
            );
        } else {
            for p in problems {
                failures.push(format!("{}: {p}", case.name));
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} microtest payload mismatches:\n  {}",
        failures.len(),
        failures.join("\n  "),
    );
}

#[test]
fn every_microtest_boots_bit_identically_twice() {
    let mut failures: Vec<String> = Vec::new();
    for case in CASES {
        let a = run_observation(case, "det_a");
        let b = run_observation(case, "det_b");
        // Anti-vacuity floor: two boots that both retired nothing and
        // printed nothing are trivially equal, so the equalities below
        // are only evidence once each run carries a payload.
        if a.metadata.steps.is_none_or(|s| s == 0) {
            failures.push(format!(
                "{}: first boot retired no steps ({:?}); the comparison would be vacuous",
                case.name, a.metadata.steps,
            ));
            continue;
        }
        if a.tty_log.is_empty() {
            failures.push(format!(
                "{}: first boot produced no TTY; the comparison would be vacuous",
                case.name
            ));
            continue;
        }
        if a.metadata.steps != b.metadata.steps {
            failures.push(format!(
                "{}: step count moved between boots ({:?} then {:?})",
                case.name, a.metadata.steps, b.metadata.steps,
            ));
        }
        if a.tty_log != b.tty_log {
            failures.push(format!("{}: TTY payload differs between boots", case.name));
        }
        if a.memory_regions != b.memory_regions {
            failures.push(format!(
                "{}: observed memory differs between boots",
                case.name
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} determinism failures:\n  {}",
        failures.len(),
        failures.join("\n  "),
    );
}

/// A microtest that boots under `boot run` but is absent from [`CASES`]
/// is watched by nothing -- the state that let four of them sit broken.
#[test]
fn every_bootable_microtest_is_covered_by_the_table() {
    let micro = workspace_root().join("tests").join("micro");
    let mut bootable: Vec<String> = Vec::new();
    let mut scenario_only: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(&micro).expect("read tests/micro") {
        // An unreadable entry is a hard error: dropping it would take a
        // microtest out of the coverage set this gate is here to keep
        // whole.
        let entry =
            entry.unwrap_or_else(|e| panic!("read_dir entry under {}: {e}", micro.display()));
        let manifest = entry.path().join("manifest.toml");
        if !manifest.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
        let name = entry.file_name().to_string_lossy().into_owned();
        // The SPU scenario manifests carry `[cellgov] scenario = ...`
        // and no title table; they are driven by the compare harness.
        if declares_a_cellgov_title(&manifest, &text) {
            bootable.push(name);
        } else {
            scenario_only.push(name);
        }
    }
    bootable.sort();
    // Floor on the population, not just the set equality: two empty
    // lists also compare equal, which is what a renamed marker table or
    // an emptied `CASES` would produce.
    assert!(
        !bootable.is_empty(),
        "no manifest under {} declares a title; either the corpus moved \
         or the layouts this scan accepts are stale",
        micro.display()
    );
    // The other half of the same guard: a predicate that answers yes to
    // everything -- keying on "a [cellgov] table exists" is the easy way
    // to write one -- partitions nothing.
    assert!(
        !scenario_only.is_empty(),
        "every manifest under {} was classified bootable; the compare-harness \
         scenario manifests must land on the other side of this predicate",
        micro.display()
    );
    let mut covered: Vec<String> = CASES.iter().map(|c| c.name.to_string()).collect();
    covered.sort();
    assert_eq!(
        bootable, covered,
        "tests/micro holds bootable manifests that this gate does not check \
         (or names cases whose manifest is gone); add the missing ones to CASES",
    );
}

#[test]
fn a_root_level_title_table_declares_a_bootable_microtest() {
    let p = std::path::Path::new("micro/manifest.toml");
    assert!(declares_a_cellgov_title(
        p,
        "[title]\nshort_name = \"x\"\n\n[checkpoint]\nkind = \"process-exit\"\n"
    ));
    assert!(declares_a_cellgov_title(
        p,
        "[cellgov.title]\nshort_name = \"x\"\n"
    ));
}

#[test]
fn a_spaced_header_declares_a_bootable_microtest() {
    let p = std::path::Path::new("micro/manifest.toml");
    assert!(declares_a_cellgov_title(
        p,
        "[ cellgov . title ]\nshort_name = \"x\"\n"
    ));
}

#[test]
fn a_scenario_manifest_does_not_declare_a_bootable_microtest() {
    let p = std::path::Path::new("micro/manifest.toml");
    assert!(!declares_a_cellgov_title(
        p,
        "[cellgov]\nscenario = \"mailbox\"\n"
    ));
    // A table header quoted inside a comment is prose, not a
    // declaration.
    assert!(!declares_a_cellgov_title(
        p,
        "# No [cellgov.title] table here.\n[test]\nname = \"x\"\n"
    ));
}

/// The corpus's own scenario manifest, by name: its `[cellgov]` table
/// carries an inline `scenario_args` sub-table, which a predicate keyed
/// on "the manifest has a `[cellgov]` table" or on any nested table
/// under it would read as a title.
#[test]
fn the_corpus_scenario_manifest_is_not_classified_bootable() {
    let manifest = workspace_root()
        .join("tests")
        .join("micro")
        .join("spu_mailbox_write")
        .join("manifest.toml");
    let text = std::fs::read_to_string(&manifest)
        .unwrap_or_else(|e| panic!("read {}: {e}", manifest.display()));
    assert!(!declares_a_cellgov_title(&manifest, &text));
}

/// A table of all-`Any` fields would boot every microtest and assert
/// nothing, reporting green while checking no behaviour at all.
#[test]
fn the_expectation_table_pins_real_values() {
    assert!(!CASES.is_empty(), "the expectation table is empty");
    for case in CASES {
        assert!(
            !case.fields.is_empty(),
            "{}: no fields declared; the payload length check would pass vacuously",
            case.name
        );
        let pinned = case
            .fields
            .iter()
            .filter(|(_, e)| matches!(e, Exact(_) | NonZero))
            .count();
        assert!(
            pinned >= 2,
            "{}: only {pinned} pinned field(s); a case that checks at most a status word \
             is not evidence the behaviour under test happened",
            case.name,
        );
        assert!(
            matches!(case.fields[0].1, Exact(0)),
            "{}: first payload word must be the status, pinned to 0",
            case.name
        );
    }
}

/// Each case's directory must actually hold the built ELF, so a
/// half-built corpus fails naming the case rather than at whichever
/// boot happens to run first.
#[test]
fn every_case_has_a_built_elf() {
    for case in CASES {
        let build = workspace_root()
            .join("tests")
            .join("micro")
            .join(case.name)
            .join("build");
        let has_elf = std::fs::read_dir(&build)
            .map(|d| {
                d.flatten()
                    .any(|e| e.path().extension().is_some_and(|x| x == "elf"))
            })
            .unwrap_or(false);
        assert!(
            has_elf,
            "{}: no .elf under {}\nthe microtests feature declares the corpus built; \
             build it with tests/micro/{}/build.sh",
            case.name,
            build.display(),
            case.name,
        );
    }
}

/// Anchors the frame reader against a hand-built frame, so a parser
/// bug cannot make every case fail in the same confusing way.
#[test]
fn the_cgov_reader_finds_a_frame_behind_a_verdict_line() {
    let case = &CASES[0];
    let mut tty = b"RSX_SOMETHING: PASS\n".to_vec();
    tty.extend_from_slice(b"CGOV");
    tty.extend_from_slice(&8u32.to_be_bytes());
    tty.extend_from_slice(&0xDEAD_BEEFu32.to_be_bytes());
    tty.extend_from_slice(&0x0000_0001u32.to_be_bytes());
    assert_eq!(cgov_words(case, &tty), vec![0xDEAD_BEEF, 1]);
}
