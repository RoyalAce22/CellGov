//! Scenario registry: named testkit scenarios, LV2 microtest
//! fixtures, and the deterministic `report` formatter that the
//! bare-scenario dispatch path uses.
//!
//! Holds the `SCENARIOS` and `MICROTESTS` lists as the single
//! source of truth for "what the CLI knows about"; the usage
//! banner and dispatch/validation paths all read from here
//! instead of keeping parallel copies.

use cellgov_testkit::fixtures::{self, ScenarioFixture};
use cellgov_testkit::runner::{run, ScenarioOutcome, ScenarioResult};

use super::exit::load_file_or_die;

/// Supported scenario names and the fixture factories that produce them.
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
    Some((label, run(fixture)))
}

/// Return a closure that builds a fresh ScenarioFixture for the named
/// scenario. Used by the compare command which needs to run the
/// scenario twice for the determinism check.
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

/// Build a ScenarioFixture for an LV2-driven ELF microtest.
///
/// Reads PPU and SPU ELF binaries from `tests/micro/<name>/build/`,
/// then constructs a fixture that boots the PPU, which drives SPU
/// creation via LV2 syscalls.
pub(crate) fn build_lv2_fixture(name: &str) -> ScenarioFixture {
    use cellgov_mem::{ByteRange, GuestAddr};
    use cellgov_ppu::PpuExecutionUnit;
    use cellgov_spu::{loader as spu_loader, SpuExecutionUnit};
    use cellgov_time::Budget;
    use std::cell::RefCell;
    use std::rc::Rc;

    let base = format!("tests/micro/{name}/build");
    let ppu_elf = load_file_or_die(&format!("{base}/{name}.elf"));
    let spu_elf = load_file_or_die(&format!("{base}/spu_main.elf"));

    let mem_size = 0x1002_0000usize;
    let stack_top = (mem_size as u64) - 0x1000;
    let primed: Rc<RefCell<Option<cellgov_ppu::state::PpuState>>> = Rc::new(RefCell::new(None));
    let primed_seed = Rc::clone(&primed);
    let primed_reg = Rc::clone(&primed);

    ScenarioFixture::builder()
        .memory_size(mem_size)
        .budget(Budget::new(100_000))
        .max_steps(10_000)
        .seed_memory(move |mem| {
            let li_r11_22: u32 = (14 << 26) | (11 << 21) | 22;
            let sc: u32 = 0x4400_0002;
            let stub_range = ByteRange::new(GuestAddr::new(0), 8).unwrap();
            let mut stub_bytes = Vec::with_capacity(8);
            stub_bytes.extend_from_slice(&li_r11_22.to_be_bytes());
            stub_bytes.extend_from_slice(&sc.to_be_bytes());
            mem.apply_commit(stub_range, &stub_bytes).unwrap();

            let mut state = cellgov_ppu::state::PpuState::new();
            cellgov_ppu::loader::load_ppu_elf(&ppu_elf, mem, &mut state).unwrap();
            state.gpr[1] = stack_top;
            state.lr = 0;
            *primed_seed.borrow_mut() = Some(state);
        })
        .register(move |rt| {
            rt.lv2_host_mut()
                .content_store_mut()
                .register(b"/app_home/spu_main.elf", spu_elf.clone());

            rt.set_spu_factory(move |id, init| {
                let mut unit = SpuExecutionUnit::new(id);
                spu_loader::load_spu_elf(&init.ls_bytes, unit.state_mut()).unwrap();
                unit.state_mut().pc = init.entry_pc;
                unit.state_mut().set_reg_word_splat(1, init.stack_ptr);
                unit.state_mut().set_reg_word_splat(3, init.args[0] as u32);
                unit.state_mut().set_reg_word_splat(4, init.args[1] as u32);
                unit.state_mut().set_reg_word_splat(5, init.args[2] as u32);
                unit.state_mut().set_reg_word_splat(6, init.args[3] as u32);
                Box::new(unit)
            });

            let ppu_state = primed_reg.borrow_mut().take().unwrap();
            rt.registry_mut().register_with(|id| {
                let mut unit = PpuExecutionUnit::new(id);
                *unit.state_mut() = ppu_state;
                unit
            });
        })
        .build()
}

/// Region specs for each microtest: (symbol_name, [(region_name, offset, size)]).
pub(crate) fn microtest_region_defs(name: &str) -> (&str, Vec<(&str, u64, u64)>) {
    match name {
        "barrier_wakeup" => ("buf", vec![("spu0_result", 0, 8), ("spu1_result", 16, 8)]),
        "mailbox_roundtrip" => ("result", vec![("result", 0, 8)]),
        "atomic_reservation" => ("buf", vec![("header", 0, 8), ("data", 16, 128)]),
        _ => super::exit::die(&format!("no region defs for microtest: {name}")),
    }
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
mod tests {
    use super::*;
    use cellgov_compare::observe_with_determinism_check;
    use cellgov_explore::ExplorationConfig;
    use cellgov_trace::TraceReader;

    #[test]
    fn every_named_scenario_runs_successfully() {
        for name in SCENARIOS {
            let (label, result) =
                run_scenario(name).unwrap_or_else(|| panic!("scenario {name} not found"));
            assert_eq!(
                result.outcome,
                ScenarioOutcome::Stalled,
                "scenario {label} did not stall cleanly"
            );
            assert!(result.steps_taken > 0, "scenario {label} took zero steps");
        }
    }

    #[test]
    fn unknown_scenario_returns_none() {
        assert!(run_scenario("nonexistent").is_none());
    }

    #[test]
    fn report_is_deterministic_across_runs() {
        let (l1, r1) = run_scenario("fairness").unwrap();
        let (l2, r2) = run_scenario("fairness").unwrap();
        assert_eq!(report(l1, &r1), report(l2, &r2));
    }

    #[test]
    fn dump_does_not_panic_for_any_scenario() {
        for name in SCENARIOS {
            let (_, result) =
                run_scenario(name).unwrap_or_else(|| panic!("scenario {name} not found"));
            let records: Vec<_> = TraceReader::new(&result.trace_bytes)
                .enumerate()
                .map(|(i, r)| {
                    r.unwrap_or_else(|e| {
                        panic!("scenario {name} decode failed at record index {i}: {e:?}")
                    })
                })
                .collect();
            assert!(
                !records.is_empty(),
                "scenario {name} produced no trace records"
            );
        }
    }

    #[test]
    fn report_includes_sync_hash_field() {
        let (label, result) = run_scenario("isa").unwrap();
        let r = report(label, &result);
        assert!(r.contains("sync_hash: 0x"));
    }

    #[test]
    fn compare_succeeds_for_every_scenario() {
        for name in SCENARIOS {
            let factory =
                scenario_factory(name).unwrap_or_else(|| panic!("scenario {name} not found"));
            let result = observe_with_determinism_check(&factory, &[]);
            assert!(
                result.is_ok(),
                "compare failed for {name}: {:?}",
                result.err()
            );
        }
    }

    #[test]
    fn scenario_factory_accepts_full_names() {
        assert!(scenario_factory("mailbox_send").is_some());
        assert!(scenario_factory("dma_block_unblock").is_some());
        assert!(scenario_factory("fake_isa").is_some());
        assert!(scenario_factory("round_robin_fairness").is_some());
        assert!(scenario_factory("write_conflict").is_some());
        assert!(scenario_factory("signal_update").is_some());
        assert!(scenario_factory("mailbox_roundtrip").is_some());
    }

    #[test]
    fn scenario_factory_returns_none_for_unknown() {
        assert!(scenario_factory("nonexistent").is_none());
    }

    #[test]
    fn explore_runs_for_multi_unit_scenarios() {
        for name in &["fairness", "conflict", "mailbox"] {
            let factory =
                scenario_factory(name).unwrap_or_else(|| panic!("scenario {name} not found"));
            let config = ExplorationConfig::default();
            let result = cellgov_explore::explore(|| factory().build_runtime(), &config);
            assert!(
                result.is_some(),
                "scenario {name} should have branching points"
            );
        }
    }

    #[test]
    fn explore_single_unit_returns_none() {
        let factory = scenario_factory("isa").expect("isa scenario exists");
        let config = ExplorationConfig::default();
        let result = cellgov_explore::explore(|| factory().build_runtime(), &config);
        assert!(result.is_none(), "single-unit isa has no branching points");
    }

    #[test]
    #[ignore] // ~7 min: runs 3 ELF microtests with full exploration
    fn explore_micro_runs_for_elf_microtests() {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let repo_root = std::path::Path::new(&manifest_dir)
            .parent()
            .unwrap()
            .parent()
            .unwrap();
        let config = ExplorationConfig::default();
        for name in MICROTESTS {
            let base = repo_root.join(format!("tests/micro/{name}/build"));
            let ppu_path = base.join(format!("{name}.elf"));
            let spu_path = base.join("spu_main.elf");
            if !ppu_path.exists() || !spu_path.exists() {
                continue;
            }
            let prev = std::env::current_dir().unwrap();
            std::env::set_current_dir(repo_root).unwrap();
            let result =
                cellgov_explore::explore(|| build_lv2_fixture(name).build_runtime(), &config);
            std::env::set_current_dir(&prev).unwrap();
            assert!(
                result.is_some(),
                "microtest {name} should have branching points"
            );
            let r = result.unwrap();
            assert_eq!(
                r.outcome,
                cellgov_explore::OutcomeClass::ScheduleStable,
                "microtest {name} should be schedule-stable"
            );
        }
    }
}
