//! Scenario registry: named testkit scenarios, LV2 microtest
//! fixtures, and the deterministic `report` formatter used by the
//! bare-scenario dispatch path.

use cellgov_testkit::fixtures::{self, ScenarioFixture};
use cellgov_testkit::runner::{run, ScenarioOutcome, ScenarioResult};

use super::exit::CommandError;
use super::self_load::load_file;

pub(crate) fn run_scenario(name: &str) -> Option<(&str, ScenarioResult)> {
    let (label, fixture) = match name {
        "fairness" => (
            "round-robin-fairness(3 units, 5 steps each)",
            fixtures::round_robin_fairness_scenario(3, 5),
        ),
        "conflict" => (
            "write-conflict(3 steps each)",
            fixtures::write_conflict_scenario(3),
        ),
        "mailbox" => (
            "mailbox-roundtrip(command=0x42)",
            fixtures::mailbox_roundtrip_scenario(0x42),
        ),
        "dma" => ("dma-block-unblock", fixtures::dma_block_unblock_scenario()),
        "send" => (
            "mailbox-send(5 messages)",
            fixtures::mailbox_send_scenario(5),
        ),
        "signal" => ("signal-update(4 bits)", fixtures::signal_update_scenario(4)),
        "isa" => ("fake-isa-integration", fixtures::fake_isa_scenario()),
        _ => return None,
    };
    let result = run(fixture);
    // Both `scenario run` and `scenario dump` reach the runtime through
    // here, so reporting once covers each of them.
    super::compare::report_first_invariant_break(result.first_invariant_break.as_deref());
    Some((label, result))
}

/// Return a closure that builds a fresh ScenarioFixture for the named
/// scenario. `diff compare` uses this to run the scenario twice for the
/// determinism check.
pub(crate) fn scenario_factory(name: &str) -> Option<Box<dyn Fn() -> ScenarioFixture>> {
    let factory: Box<dyn Fn() -> ScenarioFixture> = match name {
        "fairness" | "round_robin_fairness" => {
            Box::new(|| fixtures::round_robin_fairness_scenario(3, 5))
        }
        "conflict" | "write_conflict" => Box::new(|| fixtures::write_conflict_scenario(3)),
        "mailbox" | "mailbox_roundtrip" => Box::new(|| fixtures::mailbox_roundtrip_scenario(0x42)),
        "dma" | "dma_block_unblock" => Box::new(fixtures::dma_block_unblock_scenario),
        "send" | "mailbox_send" => Box::new(|| fixtures::mailbox_send_scenario(5)),
        "signal" | "signal_update" => Box::new(|| fixtures::signal_update_scenario(4)),
        "isa" | "fake_isa" => Box::new(fixtures::fake_isa_scenario),
        _ => return None,
    };
    Some(factory)
}

pub(crate) const SCENARIOS: &[&str] = &[
    "fairness", "conflict", "mailbox", "dma", "send", "signal", "isa",
];

pub(crate) const MICROTESTS: &[&str] =
    &["barrier_wakeup", "mailbox_roundtrip", "atomic_reservation"];

/// Builds an LV2-driven ELF microtest fixture.
///
/// Reads PPU and SPU ELF binaries from `tests/micro/<name>/build/`.
///
/// # Errors
///
/// Returns an error if either ELF cannot be read.
pub(crate) fn build_lv2_fixture(name: &str) -> Result<ScenarioFixture, CommandError> {
    build_lv2_fixture_under(std::path::Path::new("."), name)
}

/// Builds and validates an LV2-driven ELF microtest runtime.
///
/// # Errors
///
/// Returns an error if the fixture cannot be built or if runtime
/// construction produced no execution units.
pub(crate) fn build_lv2_runtime(name: &str) -> Result<cellgov_core::Runtime, CommandError> {
    let rt = build_lv2_fixture(name)?.build_runtime();
    if rt.registry().is_empty() {
        return Err(CommandError::failed(format!(
            "microtest {name}: runtime construction produced no execution units"
        )));
    }
    Ok(rt)
}

/// Builds a microtest fixture under an explicit input root.
///
/// The explicit root avoids changes to the process-wide working directory.
///
/// # Errors
///
/// See [`build_lv2_fixture`].
pub(crate) fn build_lv2_fixture_under(
    root: &std::path::Path,
    name: &str,
) -> Result<ScenarioFixture, CommandError> {
    let base = root.join(format!("tests/micro/{name}/build"));
    let ppu_elf = load_file(&base.join(format!("{name}.elf")).to_string_lossy())?;
    let spu_elf = load_file(&base.join("spu_main.elf").to_string_lossy())?;
    Ok(fixtures::lv2_driven_scenario(
        ppu_elf,
        spu_elf,
        cellgov_time::Budget::new(100_000),
        10_000,
        Box::new(cellgov_boot::prepare::spu_unit),
    ))
}

type MicrotestRegion = (&'static str, u64, u64);
type MicrotestRegionDefinition = (&'static str, Vec<MicrotestRegion>);

/// Returns the symbol and region specifications for a microtest.
///
/// # Errors
///
/// Returns an error if `name` is not registered.
pub(crate) fn microtest_region_defs(name: &str) -> Result<MicrotestRegionDefinition, CommandError> {
    Ok(match name {
        "barrier_wakeup" => ("buf", vec![("spu0_result", 0, 8), ("spu1_result", 16, 8)]),
        "mailbox_roundtrip" => ("result", vec![("result", 0, 8)]),
        "atomic_reservation" => ("buf", vec![("header", 0, 8), ("data", 16, 128)]),
        _ => {
            return Err(CommandError::failed(format!(
                "no region defs for microtest: {name}"
            )))
        }
    })
}

/// Format a [`ScenarioResult`] as a deterministic, ASCII-only summary.
pub(crate) fn report(name: &str, result: &ScenarioResult) -> String {
    let outcome = match result.outcome {
        ScenarioOutcome::Stalled => "Stalled",
        ScenarioOutcome::MaxStepsExceeded => "MaxStepsExceeded",
    };
    format!(
        "scenario: {name}\noutcome: {outcome}\nsteps_taken: {steps}\ntrace_bytes: {bytes}\nmemory_hash: 0x{mem:016x}\nstatus_hash: 0x{status:016x}\nsync_hash: 0x{sync:016x}",
        steps = result.steps_taken,
        bytes = result.trace_bytes.len(),
        mem = result.final_memory_hash.raw(),
        status = result.final_unit_status_hash.raw(),
        sync = result.final_sync_hash.raw(),
    )
}

#[cfg(test)]
#[path = "tests/scenarios_tests.rs"]
mod tests;
