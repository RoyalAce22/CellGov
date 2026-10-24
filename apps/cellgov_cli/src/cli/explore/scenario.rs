//! Schedule exploration over a testkit scenario or an LV2-driven ELF
//! microtest.

use cellgov_explore::{
    ExplorationConfig, ExplorationResult, MemoryRegionSpec, OracleRegions, OracleVerdict,
};
use cellgov_testkit::fixtures::ScenarioFixture;

use crate::cli::compare::{
    load_observations_from_dir, report_first_invariant_break, report_first_mfc_exception,
};
use crate::cli::exit::{CommandError, CommandExitCode};
use crate::cli::parse::OutputFormat;
use crate::cli::scenarios::{build_lv2_runtime, microtest_region_defs};
use crate::cli::self_load::load_file;

pub(super) fn run_explore(
    factory: &dyn Fn() -> ScenarioFixture,
    name: &str,
    format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    let config = ExplorationConfig::default();
    let result = cellgov_explore::explore(|| factory().build_runtime(), &config);
    Ok(report_exploration("scenario", name, result, format))
}

/// Explore a registered microtest; the caller checks the name.
pub(super) fn run_explore_micro(
    name: &str,
    format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    let rt = build_lv2_runtime(name)?;
    let result = cellgov_explore::explore(move || rt, &ExplorationConfig::default());
    Ok(report_exploration("microtest", name, result, format))
}

/// Print an exploration of the `label` target `name` and give its exit:
/// a schedule-sensitive outcome fails.
fn report_exploration(
    label: &str,
    name: &str,
    result: Option<ExplorationResult>,
    format: OutputFormat,
) -> CommandExitCode {
    let Some(r) = result else {
        println!("{label}: {name}");
        println!("outcome: no branching points (single-unit or trivial)");
        return CommandExitCode::SUCCESS;
    };
    report_first_invariant_break(r.first_invariant_break.as_deref());
    report_first_mfc_exception(r.first_mfc_exception.as_deref());
    match format {
        OutputFormat::Human => {
            println!("{label}: {name}");
            print!("{}", cellgov_explore::report::format_human(&r));
        }
        OutputFormat::Json => {
            println!("{}", cellgov_explore::report::format_json(&r));
        }
    }
    CommandExitCode::new(
        if r.outcome == cellgov_explore::OutcomeClass::ScheduleSensitive {
            crate::cli::exit_codes::FAILED
        } else {
            0
        },
    )
}

/// Explore a registered microtest against the observations in
/// `observations_dir`; the caller checks the name.
pub(super) fn run_explore_micro_oracle(
    name: &str,
    observations_dir: &str,
    format: OutputFormat,
) -> Result<CommandExitCode, CommandError> {
    let baselines = load_observations_from_dir(observations_dir)
        .map_err(|error| CommandError::failed(error.to_string()))?;
    if baselines.is_empty() {
        return Err(CommandError::failed(format!(
            "no observation .json files found in {observations_dir}"
        )));
    }

    let region_specs = oracle_region_specs(name)?;

    let rt = build_lv2_runtime(name)?;
    let result = cellgov_explore::explore_with_regions(
        move || rt,
        &ExplorationConfig::default(),
        &region_specs,
    );

    let Some(r) = result else {
        println!("microtest: {name}");
        println!("outcome: no branching points");
        println!("oracle_verdict: NOT COMPARED -- no branching points to explore");
        return Ok(CommandExitCode::SUCCESS);
    };

    report_first_invariant_break(r.exploration.first_invariant_break.as_deref());
    report_first_mfc_exception(r.exploration.first_mfc_exception.as_deref());

    // An unresolved capture (see `CapturedRegion::resolved`) holds
    // empty bytes regardless of what the run produced, so comparing it
    // would report a harness fault as an oracle mismatch.
    let unresolved = unresolved_region_names(&r.baseline, &r.alternates);
    if !unresolved.is_empty() {
        return Err(CommandError::failed(format!(
            "explore micro {name}: {} region capture(s) could not be read from committed memory: {}\n\
             a spec that does not resolve makes every verdict below it meaningless; \
             fix the region spec or the microtest before reading one",
            unresolved.len(),
            unresolved.join(", "),
        )));
    }

    let observations: Vec<OracleRegions<'_>> = baselines
        .iter()
        .map(|oracle| {
            oracle
                .memory_regions
                .iter()
                .map(|region| (region.name.as_str(), region.data.as_slice()))
                .collect()
        })
        .collect();
    let verdict = r.verdict(&observations);

    match format {
        OutputFormat::Human => {
            print_oracle_human(name, &r.exploration, baselines.len(), &verdict);
        }
        OutputFormat::Json => {
            println!(
                "{}",
                oracle_json(&r.exploration, baselines.len(), &verdict)?
            );
        }
    }

    Ok(CommandExitCode::new(if verdict.all_match() {
        0
    } else {
        crate::cli::exit_codes::FAILED
    }))
}

/// The memory regions the oracle compares for microtest `name`, placed
/// at the address of its ELF's region symbol.
fn oracle_region_specs(name: &str) -> Result<Vec<MemoryRegionSpec>, CommandError> {
    let (symbol, region_defs) = microtest_region_defs(name)?;
    let base = format!("tests/micro/{name}/build");
    let ppu_elf = load_file(&format!("{base}/{name}.elf"))?;
    let base_addr = cellgov_ppu::loader::find_symbol(&ppu_elf, symbol).ok_or_else(|| {
        CommandError::failed(format!("symbol '{symbol}' not found in {base}/{name}.elf"))
    })?;

    Ok(region_defs
        .iter()
        .map(|(rname, offset, size)| MemoryRegionSpec {
            name: (*rname).into(),
            // Every registered microtest is single-process.
            space: cellgov_core::AddressSpaceId::BOOT,
            addr: base_addr + offset,
            size: *size,
        })
        .collect())
}

fn print_oracle_human(
    name: &str,
    exploration: &ExplorationResult,
    baselines: usize,
    verdict: &OracleVerdict,
) {
    println!("microtest: {name}");
    print!("{}", cellgov_explore::report::format_human(exploration));
    println!("oracle_baselines: {baselines}");
    println!("baseline_matches_oracle: {}", verdict.baseline_matches);
    for (i, m) in verdict.alternate_matches.iter().enumerate() {
        if !m {
            println!("  schedule {i}: ORACLE MISMATCH");
        }
    }
    if verdict.all_match() {
        println!("oracle_verdict: all schedules match oracle");
    } else if verdict.any_match() {
        println!("oracle_verdict: PARTIAL -- some schedules diverge from oracle");
    } else {
        println!("oracle_verdict: NONE -- no schedule matches oracle");
    }
}

fn oracle_json(
    exploration: &ExplorationResult,
    baselines: usize,
    verdict: &OracleVerdict,
) -> Result<String, CommandError> {
    let exploration = serde_json::from_str::<serde_json::Value>(
        &cellgov_explore::report::format_json(exploration),
    )
    .map_err(|error| CommandError::failed(format!("explore micro: decode report JSON: {error}")))?;
    let json = serde_json::json!({
        "exploration": exploration,
        "oracle": {
            "baselines_count": baselines,
            "baseline_matches": verdict.baseline_matches,
            "alternate_matches": verdict.alternate_matches,
            "all_match": verdict.all_match(),
            "any_match": verdict.any_match(),
        },
    });
    serde_json::to_string_pretty(&json)
        .map_err(|error| CommandError::failed(format!("explore micro: encode JSON: {error}")))
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

#[cfg(test)]
#[path = "tests/scenario_tests.rs"]
mod tests;
