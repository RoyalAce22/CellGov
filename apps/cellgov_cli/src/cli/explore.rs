//! `explore` subcommand: bounded schedule exploration for both
//! testkit scenarios and LV2-driven ELF microtests.

use cellgov_explore::ExplorationConfig;
use cellgov_testkit::fixtures::ScenarioFixture;

use super::compare::load_observations_from_dir;
use super::exit::{die, load_file_or_die};
use super::parse::{ExploreArgs, ExploreCommand, OutputFormat};
use super::scenarios::{build_lv2_fixture, microtest_region_defs, scenario_factory, MICROTESTS};

pub(crate) fn run(args: &ExploreArgs, format: OutputFormat, scenarios_list: &[&str]) {
    match (&args.micro, &args.scenario) {
        (
            Some(ExploreCommand::Micro {
                name,
                observations_dir,
            }),
            _,
        ) => {
            if !MICROTESTS.contains(&name.as_str()) {
                die(&format!(
                    "unknown microtest: {name}
available: {}",
                    MICROTESTS.join(", ")
                ));
            }
            match observations_dir {
                Some(dir) => run_explore_micro_oracle(name, &dir.display().to_string(), format),
                None => run_explore_micro(name, format),
            }
        }
        (None, Some(target)) => match scenario_factory(target) {
            Some(factory) => run_explore(&factory, target, format),
            None => die(&format!(
                "unknown scenario: {target}
available: {}",
                scenarios_list.join(", ")
            )),
        },
        // clap requires the positional unless the subcommand is
        // present, so no argv reaches this arm.
        (None, None) => die("explore: no scenario or micro-test named"),
    }
}

fn run_explore(factory: &dyn Fn() -> ScenarioFixture, name: &str, format: OutputFormat) {
    let config = ExplorationConfig::default();
    let result = cellgov_explore::explore(|| factory().build_runtime(), &config);
    match result {
        Some(r) => {
            match format {
                OutputFormat::Human => {
                    println!("scenario: {name}");
                    print!("{}", cellgov_explore::report::format_human(&r));
                }
                OutputFormat::Json => {
                    println!("{}", cellgov_explore::report::format_json(&r));
                }
            }
            if r.outcome == cellgov_explore::OutcomeClass::ScheduleSensitive {
                std::process::exit(1);
            }
        }
        None => {
            println!("scenario: {name}");
            println!("outcome: no branching points (single-unit or trivial)");
        }
    }
}

fn run_explore_micro(name: &str, format: OutputFormat) {
    if !MICROTESTS.contains(&name) {
        die(&format!(
            "unknown microtest: {name}\navailable: {}",
            MICROTESTS.join(", ")
        ));
    }
    let config = ExplorationConfig::default();
    let result = cellgov_explore::explore(|| build_lv2_fixture(name).build_runtime(), &config);
    match result {
        Some(r) => {
            match format {
                OutputFormat::Human => {
                    println!("microtest: {name}");
                    print!("{}", cellgov_explore::report::format_human(&r));
                }
                OutputFormat::Json => {
                    println!("{}", cellgov_explore::report::format_json(&r));
                }
            }
            if r.outcome == cellgov_explore::OutcomeClass::ScheduleSensitive {
                std::process::exit(1);
            }
        }
        None => {
            println!("microtest: {name}");
            println!("outcome: no branching points (single-unit or trivial)");
        }
    }
}

fn run_explore_micro_oracle(name: &str, observations_dir: &str, format: OutputFormat) {
    if !MICROTESTS.contains(&name) {
        die(&format!(
            "unknown microtest: {name}\navailable: {}",
            MICROTESTS.join(", ")
        ));
    }

    let baselines = load_observations_from_dir(observations_dir);
    if baselines.is_empty() {
        die(&format!(
            "no observation .json files found in {observations_dir}"
        ));
    }

    let (symbol, region_defs) = microtest_region_defs(name);
    let base = format!("tests/micro/{name}/build");
    let ppu_elf = load_file_or_die(&format!("{base}/{name}.elf"));
    let base_addr = cellgov_ppu::loader::find_symbol(&ppu_elf, symbol)
        .unwrap_or_else(|| die(&format!("symbol '{symbol}' not found in {base}/{name}.elf")));

    let region_specs: Vec<cellgov_explore::MemoryRegionSpec> = region_defs
        .iter()
        .map(|(rname, offset, size)| cellgov_explore::MemoryRegionSpec {
            name: (*rname).into(),
            // Every registered microtest is single-process.
            space: cellgov_core::AddressSpaceId::BOOT,
            addr: base_addr + offset,
            size: *size,
        })
        .collect();

    let config = ExplorationConfig::default();
    let result = cellgov_explore::explore_with_regions(
        || build_lv2_fixture(name).build_runtime(),
        &config,
        &region_specs,
    );

    let Some(r) = result else {
        println!("microtest: {name}");
        println!("outcome: no branching points");
        // No schedules were explored, so nothing was held against the
        // oracle. Say so rather than letting a silent exit 0 read as
        // a passing oracle check.
        println!("oracle_verdict: NOT COMPARED -- no branching points to explore");
        return;
    };

    // An unresolved capture (see `CapturedRegion::resolved`) holds
    // empty bytes regardless of what the run produced, so comparing it
    // would report a harness fault as an oracle mismatch.
    let unresolved = unresolved_region_names(&r.baseline, &r.alternates);
    if !unresolved.is_empty() {
        die(&format!(
            "explore micro {name}: {} region capture(s) could not be read from committed memory: {}\n\
             a spec that does not resolve makes every verdict below it meaningless; \
             fix the region spec or the microtest before reading one",
            unresolved.len(),
            unresolved.join(", "),
        ));
    }

    let baseline_matches = compare_regions_against_oracle(&r.baseline.regions, &baselines);
    let alt_matches: Vec<bool> = r
        .alternates
        .iter()
        .map(|s| compare_regions_against_oracle(&s.regions, &baselines))
        .collect();

    let all_match = baseline_matches && alt_matches.iter().all(|m| *m);
    let any_match = baseline_matches || alt_matches.iter().any(|m| *m);

    match format {
        OutputFormat::Human => {
            println!("microtest: {name}");
            print!("{}", cellgov_explore::report::format_human(&r.exploration));
            println!("oracle_baselines: {}", baselines.len());
            println!("baseline_matches_oracle: {baseline_matches}");
            for (i, m) in alt_matches.iter().enumerate() {
                if !m {
                    println!("  schedule {i}: ORACLE MISMATCH");
                }
            }
            if all_match {
                println!("oracle_verdict: all schedules match oracle");
            } else if any_match {
                println!("oracle_verdict: PARTIAL -- some schedules diverge from oracle");
            } else {
                println!("oracle_verdict: NONE -- no schedule matches oracle");
            }
        }
        OutputFormat::Json => {
            let json = serde_json::json!({
                "exploration": serde_json::from_str::<serde_json::Value>(
                    &cellgov_explore::report::format_json(&r.exploration)
                ).expect("invariant: exploration report is float-free by construction (style doc 0.5d contract)"),
                "oracle": {
                    "baselines_count": baselines.len(),
                    "baseline_matches": baseline_matches,
                    "alternate_matches": alt_matches,
                    "all_match": all_match,
                    "any_match": any_match,
                },
            });
            println!(
                "{}",
                serde_json::to_string_pretty(&json)
                    .expect("invariant: primitive-only serde_json::Value tree always serializes (style doc 0.5d contract)"),
            );
        }
    }

    if !all_match {
        std::process::exit(1);
    }
}

/// `schedule:region` for every capture whose range could not be read,
/// baseline first then alternates in exploration order.
fn unresolved_region_names(
    baseline: &cellgov_explore::oracle::ScheduleSnapshot,
    alternates: &[cellgov_explore::oracle::ScheduleSnapshot],
) -> Vec<String> {
    let labelled = std::iter::once(("baseline".to_string(), baseline)).chain(
        alternates
            .iter()
            .enumerate()
            .map(|(i, s)| (format!("schedule {i}"), s)),
    );
    labelled
        .flat_map(|(label, snap)| {
            snap.regions
                .iter()
                .filter(|region| !region.resolved)
                .map(move |region| format!("{label}:{}", region.name))
        })
        .collect()
}

/// Returns true when a single oracle observation carries a matching
/// name and bytes for every captured region.
fn compare_regions_against_oracle(
    captured: &[cellgov_explore::oracle::CapturedRegion],
    baselines: &[cellgov_compare::Observation],
) -> bool {
    baselines.iter().any(|oracle| {
        captured.iter().all(|region| {
            oracle.memory_regions.iter().any(|oracle_region| {
                oracle_region.name == region.name && oracle_region.data == region.data
            })
        })
    })
}

#[cfg(test)]
#[path = "tests/explore_tests.rs"]
mod tests;
