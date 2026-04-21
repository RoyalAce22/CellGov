use super::*;
use cellgov_exec::ExecutionContext;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};

/// Run a PPU unit until it reaches a non-Syscall yield, handling the
/// yield-resume cycle for syscalls by feeding back `r3 = 0` (CELL_OK)
/// and re-entering. Mimics what the runtime does for `Lv2Dispatch::Immediate`.
/// Effects from all cycles are accumulated into `effects`.
/// Panics after 1000 cycles to avoid infinite loops.
fn run_to_completion(
    unit: &mut PpuExecutionUnit,
    mem: &GuestMemory,
    budget: Budget,
) -> (ExecutionStepResult, Vec<Effect>) {
    let mut received = vec![];
    let mut syscall_ret: Option<u64> = None;
    let mut all_effects = Vec::new();
    for _ in 0..1000 {
        let ctx = if let Some(code) = syscall_ret.take() {
            ExecutionContext::with_syscall_return(mem, &received, code)
        } else {
            ExecutionContext::with_received(mem, &received)
        };
        let mut effects = Vec::new();
        let result = unit.run_until_yield(budget, &ctx, &mut effects);
        if result.yield_reason == YieldReason::Syscall {
            all_effects.extend(effects);
            if let Some(args) = &result.syscall_args {
                if args[0] == 22 {
                    unit.status = UnitStatus::Finished;
                    return (
                        ExecutionStepResult {
                            yield_reason: YieldReason::Finished,
                            ..result
                        },
                        all_effects,
                    );
                }
            }
            syscall_ret = Some(0);
            received = vec![];
            continue;
        }
        all_effects.extend(effects);
        return (result, all_effects);
    }
    panic!("PPU did not terminate within 1000 resume cycles");
}

/// Place a big-endian instruction word at a byte offset in memory.
fn place_insn(mem: &mut GuestMemory, offset: usize, raw: u32) {
    let range = ByteRange::new(GuestAddr::new(offset as u64), 4).unwrap();
    mem.apply_commit(range, &raw.to_be_bytes()).unwrap();
}

#[test]
fn new_unit_is_runnable() {
    let unit = PpuExecutionUnit::new(UnitId::new(0));
    assert_eq!(unit.status(), UnitStatus::Runnable);
    assert_eq!(unit.unit_id(), UnitId::new(0));
}

#[test]
fn li_then_sc_exit() {
    let mut mem = GuestMemory::new(256);
    let li_r11_22: u32 = (14 << 26) | (11 << 21) | 22;
    place_insn(&mut mem, 0, li_r11_22);
    place_insn(&mut mem, 4, 0x4400_0002);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let (result, _effects) = run_to_completion(&mut unit, &mem, Budget::new(100));
    assert_eq!(result.yield_reason, YieldReason::Finished);
    assert_eq!(unit.status(), UnitStatus::Finished);
    assert_eq!(unit.state().gpr[11], 22);
}

#[test]
fn store_emits_shared_write_intent() {
    // li r3, 0xBEEF; stw r3, 0(r0)
    let mut mem = GuestMemory::new(256);
    let li: u32 = (14 << 26) | (3 << 21) | (0xBEEFu16 as i16 as u16 as u32);
    place_insn(&mut mem, 0, li);
    // stw r3, 128(r0) = opcode 36, rs=3, ra=0, imm=128
    let stw: u32 = (36 << 26) | (3 << 21) | 128;
    place_insn(&mut mem, 4, stw);
    // li r11, 22; sc
    let li_exit: u32 = (14 << 26) | (11 << 21) | 22;
    place_insn(&mut mem, 8, li_exit);
    place_insn(&mut mem, 12, 0x4400_0002);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let (result, effects) = run_to_completion(&mut unit, &mem, Budget::new(100));
    assert_eq!(result.yield_reason, YieldReason::Finished);

    // Should have one SharedWriteIntent for the stw
    let writes: Vec<_> = effects
        .iter()
        .filter(|e| matches!(e, cellgov_effects::Effect::SharedWriteIntent { .. }))
        .collect();
    assert_eq!(writes.len(), 1);
    if let cellgov_effects::Effect::SharedWriteIntent { range, bytes, .. } = &writes[0] {
        assert_eq!(range.start().raw(), 128);
        assert_eq!(range.length(), 4);
        // 0xBEEF sign-extended: li sign-extends 0xBEEF (negative as i16)
        // 0xBEEF as i16 = -16657, as u64 = 0xFFFF_FFFF_FFFF_BEEF
        // stw stores low 32 bits = 0xFFFF_BEEF
        assert_eq!(bytes.bytes(), &0xFFFF_BEEFu32.to_be_bytes());
    }
}

#[test]
fn load_reads_from_guest_memory() {
    let mut mem = GuestMemory::new(256);
    // Seed 0xDEADBEEF at address 128
    let range = ByteRange::new(GuestAddr::new(128), 4).unwrap();
    mem.apply_commit(range, &0xDEAD_BEEFu32.to_be_bytes())
        .unwrap();
    // lwz r3, 128(r0)
    let lwz: u32 = (32 << 26) | (3 << 21) | 128;
    place_insn(&mut mem, 0, lwz);
    // li r11, 22; sc
    let li_exit: u32 = (14 << 26) | (11 << 21) | 22;
    place_insn(&mut mem, 4, li_exit);
    place_insn(&mut mem, 8, 0x4400_0002);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let (result, _effects) = run_to_completion(&mut unit, &mem, Budget::new(100));
    assert_eq!(result.yield_reason, YieldReason::Finished);
    assert_eq!(unit.state().gpr[3], 0xDEAD_BEEF);
}

#[test]
fn unknown_syscall_yields_with_args() {
    let mut mem = GuestMemory::new(256);
    // li r11, 999; sc
    let li: u32 = (14 << 26) | (11 << 21) | 999;
    place_insn(&mut mem, 0, li);
    place_insn(&mut mem, 4, 0x4400_0002);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::Syscall);
    let args = result.syscall_args.unwrap();
    assert_eq!(args[0], 999);
}

#[test]
fn decode_failure_faults() {
    let mut mem = GuestMemory::new(256);
    place_insn(&mut mem, 0, 0xFFFF_FFFF);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(unit.status(), UnitStatus::Faulted);
}

#[test]
fn budget_exhaustion_yields() {
    let mut mem = GuestMemory::new(256);
    // Fill with nops (ori r0, r0, 0 = 0x60000000)
    for i in (0..64).step_by(4) {
        place_insn(&mut mem, i, 0x6000_0000);
    }

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(5), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(result.consumed_budget, Budget::new(5));
    assert_eq!(unit.state().pc, 20); // 5 nops * 4 bytes
}

#[test]
fn pc_out_of_range_faults() {
    // Starting the unit with pc past the end of guest memory must
    // fault with FAULT_PC_OUT_OF_RANGE on the first fetch attempt.
    let mem = GuestMemory::new(256);
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().pc = 0x1000; // past end of 256-byte memory
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(unit.status(), UnitStatus::Faulted);
    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(FAULT_PC_OUT_OF_RANGE)),
        "fault code should be FAULT_PC_OUT_OF_RANGE"
    );
}

#[test]
fn load_out_of_range_faults() {
    // lwz r3, 0(r1) with r1 pointing outside guest memory must
    // fault with FAULT_INVALID_ADDRESS when the load is attempted.
    let mut mem = GuestMemory::new(256);
    // lwz r3, 0(r1)
    let lwz: u32 = (32 << 26) | (3 << 21) | (1 << 16);
    place_insn(&mut mem, 0, lwz);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().gpr[1] = 0x0000_0000_1000_0000; // way past 256 bytes
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(unit.status(), UnitStatus::Faulted);
    assert_eq!(
        result.fault,
        Some(FaultKind::Guest(FAULT_INVALID_ADDRESS)),
        "fault code should be FAULT_INVALID_ADDRESS"
    );
}

/// Guest address of the exit stub planted by the test harness.
/// The CRT0's outer epilogue loads LR from `r1 + 16`, which is
/// never written during the program's lifetime, so it reads the
/// initial zero in guest memory. Planting the stub at address 0
/// means the resulting `blr r0 == 0` lands on `li r11, 22; sc`
/// and cleanly calls sys_process_exit.
const EXIT_STUB_ADDR: u64 = 0;

/// Plant a 2-instruction exit stub (`li r11, 22; sc`) in guest
/// memory at EXIT_STUB_ADDR. Real PS3 thread startup sets LR to
/// a kernel trampoline that terminates the thread when main
/// returns; the test harness models that with a planted stub so
/// `blr` from the CRT0 lands somewhere useful.
fn plant_exit_stub(mem: &mut GuestMemory) {
    // li r11, 22 -> addi r11, r0, 22 = opcode 14, rt=11
    let li_r11_22: u32 = (14 << 26) | (11 << 21) | 22;
    let sc: u32 = 0x4400_0002;
    let range = ByteRange::new(GuestAddr::new(EXIT_STUB_ADDR), 8).unwrap();
    let mut bytes = Vec::with_capacity(8);
    bytes.extend_from_slice(&li_r11_22.to_be_bytes());
    bytes.extend_from_slice(&sc.to_be_bytes());
    mem.apply_commit(range, &bytes).unwrap();
}

/// Run a real microtest PPU ELF through the execution unit until
/// it halts. Returns (yield_reason, r11, consumed_budget, pc).
/// Initializes the stack pointer near the top of guest memory,
/// plants an exit stub at low memory, and points LR at the stub
/// so `blr` from the CRT0 reaches sys_process_exit. Skips
/// silently when the binary has not been built.
fn run_microtest_ppu(rel_path: &str) -> Option<(YieldReason, u64, u64, u64)> {
    let path = std::path::Path::new(rel_path);
    if !path.exists() {
        return None;
    }
    let data = std::fs::read(path).unwrap();
    let mem_size = 0x1002_0000usize;
    let mut mem = GuestMemory::new(mem_size);
    plant_exit_stub(&mut mem);
    let mut state = state::PpuState::new();
    crate::loader::load_ppu_elf(&data, &mut mem, &mut state).unwrap();
    state.gpr[1] = (mem_size as u64) - 0x1000;
    state.lr = EXIT_STUB_ADDR;

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    *unit.state_mut() = state;
    let budget = Budget::new(100_000);
    let (result, _effects) = run_to_completion(&mut unit, &mem, budget);
    Some((
        result.yield_reason,
        unit.state().gpr[11],
        result.consumed_budget.raw(),
        unit.state().pc,
    ))
}

#[test]
fn dma_completion_runs_to_process_exit() {
    // With vector register state, the widened LV2 stub range,
    // and the planted exit stub, the dma_completion CRT0 runs
    // end-to-end: through the SPU thread group lifecycle calls,
    // back from main, through the exit stub, into sys_process_exit.
    let Some((reason, r11, consumed, _pc)) =
        run_microtest_ppu("../../tests/micro/dma_completion/build/dma_completion.elf")
    else {
        return;
    };
    assert_eq!(
        reason,
        YieldReason::Finished,
        "dma_completion did not reach sys_process_exit (consumed={}, r11={})",
        consumed,
        r11
    );
    assert_eq!(r11, syscall::SYS_PROCESS_EXIT);
}

#[test]
fn spu_fixed_value_runs_to_process_exit() {
    let Some((reason, r11, consumed, _pc)) =
        run_microtest_ppu("../../tests/micro/spu_fixed_value/build/spu_fixed_value.elf")
    else {
        return;
    };
    assert_eq!(
        reason,
        YieldReason::Finished,
        "spu_fixed_value did not reach sys_process_exit (consumed={}, r11={})",
        consumed,
        r11
    );
    assert_eq!(r11, syscall::SYS_PROCESS_EXIT);
}

#[test]
fn mailbox_roundtrip_runs_to_process_exit() {
    let Some((reason, r11, consumed, _pc)) =
        run_microtest_ppu("../../tests/micro/mailbox_roundtrip/build/mailbox_roundtrip.elf")
    else {
        return;
    };
    assert_eq!(
        reason,
        YieldReason::Finished,
        "mailbox_roundtrip did not reach sys_process_exit (consumed={}, r11={})",
        consumed,
        r11
    );
}

#[test]
fn atomic_reservation_runs_to_process_exit() {
    let Some((reason, r11, consumed, _pc)) =
        run_microtest_ppu("../../tests/micro/atomic_reservation/build/atomic_reservation.elf")
    else {
        return;
    };
    assert_eq!(
        reason,
        YieldReason::Finished,
        "atomic_reservation did not reach sys_process_exit (consumed={}, r11={})",
        consumed,
        r11
    );
}

#[test]
fn barrier_wakeup_runs_to_process_exit() {
    let Some((reason, r11, consumed, _pc)) =
        run_microtest_ppu("../../tests/micro/barrier_wakeup/build/barrier_wakeup.elf")
    else {
        return;
    };
    assert_eq!(
        reason,
        YieldReason::Finished,
        "barrier_wakeup did not reach sys_process_exit (consumed={}, r11={})",
        consumed,
        r11
    );
}

/// Build a paired fixture where the PPU drives SPU creation through
/// LV2 syscalls. Only the PPU and the content store are set up; the
/// SPU factory handles unit creation when the runtime processes
/// `Lv2Dispatch::RegisterSpu`.
fn build_lv2_driven_fixture(
    ppu_elf: Vec<u8>,
    spu_elf: Vec<u8>,
    budget: Budget,
    max_steps: usize,
) -> cellgov_testkit::fixtures::ScenarioFixture {
    use cellgov_spu::{loader as spu_loader, SpuExecutionUnit};
    use cellgov_testkit::fixtures::ScenarioFixture;
    use std::cell::RefCell;
    use std::rc::Rc;

    let mem_size = 0x1002_0000usize;
    let stack_top = (mem_size as u64) - 0x1000;
    let primed: Rc<RefCell<Option<state::PpuState>>> = Rc::new(RefCell::new(None));
    let primed_seed = Rc::clone(&primed);
    let primed_reg = Rc::clone(&primed);

    ScenarioFixture::builder()
        .memory_size(mem_size)
        .budget(budget)
        .max_steps(max_steps)
        .seed_memory(move |mem| {
            let li_r11_22: u32 = (14 << 26) | (11 << 21) | 22;
            let sc: u32 = 0x4400_0002;
            let stub_range = ByteRange::new(GuestAddr::new(0), 8).unwrap();
            let mut stub_bytes = Vec::with_capacity(8);
            stub_bytes.extend_from_slice(&li_r11_22.to_be_bytes());
            stub_bytes.extend_from_slice(&sc.to_be_bytes());
            mem.apply_commit(stub_range, &stub_bytes).unwrap();

            let mut state = state::PpuState::new();
            crate::loader::load_ppu_elf(&ppu_elf, mem, &mut state).unwrap();
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

#[test]
fn spu_fixed_value_runs_through_scenario_runner() {
    // Same end-to-end exercise as the direct run, but driven by
    // the canonical testkit runner via a ScenarioFixture. This
    // verifies that the PpuExecutionUnit plugs into the standard
    // Runtime scheduler loop, that its emitted stores flow
    // through the commit pipeline, and that its `Finished` yield
    // is recognized as a terminal state.
    use cellgov_testkit::fixtures::ScenarioFixture;
    use cellgov_testkit::runner::{self, ScenarioOutcome};
    use std::cell::RefCell;
    use std::rc::Rc;

    let elf_path =
        std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_fixed_value.elf");
    if !elf_path.exists() {
        return;
    }
    let elf_data = std::fs::read(elf_path).unwrap();
    let mem_size = 0x1002_0000usize;
    let stack_top = (mem_size as u64) - 0x1000;

    // Seed-memory and register callbacks are both FnOnce and
    // cannot directly share a value. Pass the pre-loaded PpuState
    // through an Rc<RefCell<Option<_>>> so the register closure
    // can pick it up after seed_memory has parsed the ELF.
    let primed: Rc<RefCell<Option<state::PpuState>>> = Rc::new(RefCell::new(None));
    let primed_seed = Rc::clone(&primed);
    let primed_reg = Rc::clone(&primed);

    let fixture = ScenarioFixture::builder()
        .memory_size(mem_size)
        .budget(Budget::new(10_000))
        .max_steps(200)
        .seed_memory(move |mem| {
            // Plant exit stub at address 0 so a terminal `blr`
            // with LR loaded as 0 from an uninitialized linkage
            // slot lands on `li r11, 22; sc`.
            let li_r11_22: u32 = (14 << 26) | (11 << 21) | 22;
            let sc: u32 = 0x4400_0002;
            let stub_range = ByteRange::new(GuestAddr::new(0), 8).unwrap();
            let mut stub_bytes = Vec::with_capacity(8);
            stub_bytes.extend_from_slice(&li_r11_22.to_be_bytes());
            stub_bytes.extend_from_slice(&sc.to_be_bytes());
            mem.apply_commit(stub_range, &stub_bytes).unwrap();

            let mut state = state::PpuState::new();
            crate::loader::load_ppu_elf(&elf_data, mem, &mut state).unwrap();
            state.gpr[1] = stack_top;
            state.lr = 0;
            *primed_seed.borrow_mut() = Some(state);
        })
        .register(move |rt| {
            let state = primed_reg.borrow_mut().take().unwrap();
            rt.registry_mut().register_with(|id| {
                let mut unit = PpuExecutionUnit::new(id);
                *unit.state_mut() = state;
                unit
            });
        })
        .build();

    let result = runner::run(fixture);
    assert_eq!(
        result.outcome,
        ScenarioOutcome::Stalled,
        "scenario did not cleanly stall (steps={})",
        result.steps_taken
    );
    // The PPU must have actually made progress, not zero-stepped.
    assert!(
        result.steps_taken > 0,
        "PPU did not execute any steps in the scenario"
    );
}

#[test]
fn ls_to_shared_runs_to_process_exit() {
    let Some((reason, r11, consumed, _pc)) =
        run_microtest_ppu("../../tests/micro/ls_to_shared/build/ls_to_shared.elf")
    else {
        return;
    };
    assert_eq!(
        reason,
        YieldReason::Finished,
        "ls_to_shared did not reach sys_process_exit (consumed={}, r11={})",
        consumed,
        r11
    );
}

#[test]
fn snapshot_captures_state() {
    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().gpr[3] = 0xCAFE;
    unit.state_mut().pc = 0x1000;
    unit.state_mut().lr = 0x2000;
    let snap = unit.snapshot();
    assert_eq!(snap.pc, 0x1000);
    assert_eq!(snap.gpr[3], 0xCAFE);
    assert_eq!(snap.lr, 0x2000);
}

#[test]
fn spu_fixed_value_image_open_writes_handle_to_guest_memory() {
    let ppu_path =
        std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_fixed_value.elf");
    let spu_path = std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_main.elf");
    if !ppu_path.exists() || !spu_path.exists() {
        return;
    }
    let ppu_elf = std::fs::read(ppu_path).unwrap();
    let spu_elf = std::fs::read(spu_path).unwrap();

    let fixture = build_lv2_driven_fixture(ppu_elf, spu_elf, Budget::new(100_000), 10_000);
    let result = cellgov_testkit::runner::run(fixture);
    assert_eq!(
        result.outcome,
        cellgov_testkit::runner::ScenarioOutcome::Stalled,
        "scenario should complete"
    );

    // The PSL1GHT CRT0 calls sys_spu_image_open and stores the
    // sys_spu_image_t struct at a stack address. We cannot easily
    // find the exact address, but we can verify the content store
    // handle (1) appears somewhere in committed memory as a
    // big-endian u32. This confirms the dispatch path wrote the
    // struct.
    let mem = &result.final_memory;
    let handle_be = 1u32.to_be_bytes();
    let found = mem.windows(4).any(|w| w == handle_be);
    assert!(
        found,
        "expected sys_spu_image_t handle (0x00000001) in committed memory"
    );
}

#[test]
fn spu_fixed_value_lv2_driven_factory_fires_and_completes() {
    let ppu_path =
        std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_fixed_value.elf");
    let spu_path = std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_main.elf");
    if !ppu_path.exists() || !spu_path.exists() {
        return;
    }
    let ppu_elf = std::fs::read(ppu_path).unwrap();
    let spu_elf = std::fs::read(spu_path).unwrap();

    let fixture = build_lv2_driven_fixture(ppu_elf, spu_elf, Budget::new(100_000), 10_000);
    let result = cellgov_testkit::runner::run(fixture);
    assert_eq!(
        result.outcome,
        cellgov_testkit::runner::ScenarioOutcome::Stalled,
        "LV2-driven spu_fixed_value did not complete"
    );
    assert!(
        result.steps_taken > 5,
        "expected more than 5 steps, got {}",
        result.steps_taken
    );

    // The SPU DMA'd TestResult { status: 0, value: 0x1337BAAD }
    // to the PPU's result buffer. Verify the pattern exists in
    // committed memory -- the exact address depends on the ELF
    // layout, but the 8-byte payload is unique.
    let expected = [0x00, 0x00, 0x00, 0x00, 0x13, 0x37, 0xBA, 0xAD];
    let mem = &result.final_memory;
    let found = mem.windows(8).any(|w| w == expected);
    assert!(
        found,
        "expected TestResult {{status=0, value=0x1337BAAD}} in committed memory"
    );
}

/// Helper: run an LV2-driven microtest and assert the scenario
/// completes. Returns the final memory for payload checks.
fn run_lv2_driven_microtest(name: &str) -> Option<Vec<u8>> {
    let ppu_path =
        std::path::PathBuf::from(format!("../../tests/micro/{}/build/{}.elf", name, name));
    let spu_path =
        std::path::PathBuf::from(format!("../../tests/micro/{}/build/spu_main.elf", name));
    if !ppu_path.exists() || !spu_path.exists() {
        return None;
    }
    let ppu_elf = std::fs::read(&ppu_path).unwrap();
    let spu_elf = std::fs::read(&spu_path).unwrap();
    let fixture = build_lv2_driven_fixture(ppu_elf, spu_elf, Budget::new(100_000), 10_000);
    let result = cellgov_testkit::runner::run(fixture);
    assert_eq!(
        result.outcome,
        cellgov_testkit::runner::ScenarioOutcome::Stalled,
        "LV2-driven {} did not complete",
        name
    );
    assert!(
        result.steps_taken > 5,
        "{}: expected more than 5 steps, got {}",
        name,
        result.steps_taken
    );
    Some(result.final_memory)
}

/// Run an LV2-driven microtest through `observe_with_determinism_check`
/// and compare against RPCS3 baselines. Skips if ELFs or baselines are
/// missing. `symbol` is the ELF symbol whose address is the base of the
/// observable region. `region_defs` are (name, offset_from_base, size).
fn run_lv2_driven_baseline_check(microtest: &str, symbol: &str, region_defs: &[(&str, u64, u64)]) {
    let ppu_path = std::path::PathBuf::from(format!(
        "../../tests/micro/{}/build/{}.elf",
        microtest, microtest
    ));
    let spu_path = std::path::PathBuf::from(format!(
        "../../tests/micro/{}/build/spu_main.elf",
        microtest
    ));
    let baseline_dir = std::path::PathBuf::from(format!("../../baselines/{}", microtest));
    let interp_path = baseline_dir.join("rpcs3_interpreter.json");
    let llvm_path = baseline_dir.join("rpcs3_llvm.json");
    if !ppu_path.exists() || !spu_path.exists() || !interp_path.exists() || !llvm_path.exists() {
        return;
    }
    let ppu_elf = std::fs::read(&ppu_path).unwrap();
    let spu_elf = std::fs::read(&spu_path).unwrap();

    let base_addr = crate::loader::find_symbol(&ppu_elf, symbol)
        .unwrap_or_else(|| panic!("symbol '{}' not found in {}", symbol, microtest));
    let regions: Vec<cellgov_compare::RegionDescriptor> = region_defs
        .iter()
        .map(|(name, offset, size)| cellgov_compare::RegionDescriptor {
            name: (*name).into(),
            addr: base_addr + offset,
            size: *size,
        })
        .collect();

    let factory = move || {
        build_lv2_driven_fixture(
            ppu_elf.clone(),
            spu_elf.clone(),
            Budget::new(100_000),
            10_000,
        )
    };

    let cellgov_obs = cellgov_compare::observe_with_determinism_check(factory, &regions).unwrap();
    assert_eq!(
        cellgov_obs.outcome,
        cellgov_compare::ObservedOutcome::Completed,
        "{} LV2-driven scenario did not complete",
        microtest
    );

    let baselines = vec![
        cellgov_compare::baseline::load(&interp_path).unwrap(),
        cellgov_compare::baseline::load(&llvm_path).unwrap(),
    ];
    let result = cellgov_compare::compare_multi(
        &baselines,
        &cellgov_obs,
        cellgov_compare::CompareMode::Memory,
    );
    assert_eq!(
        result.classification,
        cellgov_compare::Classification::Match,
        "{} LV2-driven diverges from RPCS3: {:?}",
        microtest,
        result.cellgov_result
    );
}

#[test]
fn spu_fixed_value_lv2_baseline() {
    run_lv2_driven_baseline_check("spu_fixed_value", "result", &[("result", 0, 8)]);
}

#[test]
fn dma_completion_lv2_baseline() {
    run_lv2_driven_baseline_check(
        "dma_completion",
        "result_buf",
        &[("header", 0, 8), ("pattern", 16, 128)],
    );
}

#[test]
fn ls_to_shared_lv2_baseline() {
    run_lv2_driven_baseline_check(
        "ls_to_shared",
        "result_buf",
        &[("header", 0, 8), ("data", 16, 128)],
    );
}

#[test]
fn atomic_reservation_lv2_baseline() {
    run_lv2_driven_baseline_check(
        "atomic_reservation",
        "buf",
        &[("header", 0, 8), ("data", 16, 128)],
    );
}

#[test]
fn mailbox_roundtrip_lv2_baseline() {
    run_lv2_driven_baseline_check("mailbox_roundtrip", "result", &[("result", 0, 8)]);
}

#[test]
fn barrier_wakeup_lv2_baseline() {
    run_lv2_driven_baseline_check(
        "barrier_wakeup",
        "buf",
        &[("spu0_result", 0, 8), ("spu1_result", 16, 8)],
    );
}

#[test]
fn dma_completion_lv2_driven() {
    let mem = match run_lv2_driven_microtest("dma_completion") {
        Some(m) => m,
        None => return,
    };
    // Header: status=0, size=0x80. Data: 0xDEADBEEF repeated.
    let marker = [0xDE, 0xAD, 0xBE, 0xEF, 0xDE, 0xAD, 0xBE, 0xEF];
    assert!(
        mem.windows(8).any(|w| w == marker),
        "expected 0xDEADBEEF pattern in committed memory"
    );
}

#[test]
fn ls_to_shared_lv2_driven() {
    let mem = match run_lv2_driven_microtest("ls_to_shared") {
        Some(m) => m,
        None => return,
    };
    // First data word: 0xC0DE0000 (big-endian).
    let marker = [0xC0, 0xDE, 0x00, 0x00];
    assert!(
        mem.windows(4).any(|w| w == marker),
        "expected 0xC0DE0000 pattern in committed memory"
    );
}

#[test]
fn atomic_reservation_lv2_driven() {
    let mem = match run_lv2_driven_microtest("atomic_reservation") {
        Some(m) => m,
        None => return,
    };
    // Data region: 128 bytes of 0xBBBBBBBB.
    let marker = [0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB, 0xBB];
    assert!(
        mem.windows(8).any(|w| w == marker),
        "expected 0xBBBBBBBB data in committed memory"
    );
}

#[test]
fn barrier_wakeup_lv2_driven() {
    let mem = match run_lv2_driven_microtest("barrier_wakeup") {
        Some(m) => m,
        None => return,
    };
    // SPU 0 writes marker 0xAAAA0000, SPU 1 writes 0xBBBB0001.
    let spu0 = [0xAA, 0xAA, 0x00, 0x00];
    let spu1 = [0xBB, 0xBB, 0x00, 0x01];
    assert!(
        mem.windows(4).any(|w| w == spu0),
        "expected SPU 0 marker 0xAAAA0000 in committed memory"
    );
    assert!(
        mem.windows(4).any(|w| w == spu1),
        "expected SPU 1 marker 0xBBBB0001 in committed memory"
    );
}

#[test]
fn mailbox_roundtrip_lv2_driven() {
    let mem = match run_lv2_driven_microtest("mailbox_roundtrip") {
        Some(m) => m,
        None => return,
    };
    // PPU sends 0x42 to SPU inbound mailbox via sysSpuThreadWriteMb.
    // SPU reads it, XORs with 0xFFFFFFFF -> 0xFFFFFFBD, DMAs result.
    // TestResult: status=0, value=0xFFFFFFBD.
    let expected = [0x00, 0x00, 0x00, 0x00, 0xFF, 0xFF, 0xFF, 0xBD];
    assert!(
        mem.windows(8).any(|w| w == expected),
        "expected TestResult {{status=0, value=0xFFFFFFBD}} in committed memory"
    );
}

// -- Real game ELF loading --

#[test]
fn retail_eboot_loads_into_guest_memory() {
    let path =
        std::path::PathBuf::from("../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf");
    if !path.exists() {
        return; // skip if the fixture is not installed
    }
    let data = std::fs::read(&path).unwrap();

    let mut state = state::PpuState::new();
    // A small retail EBOOT's highest PT_LOAD in this fixture sits at
    // 0x10000000 (a PS3-standard read-only segment placement), so
    // guest memory must span up to that plus the segment footprint.
    let mut mem = GuestMemory::new(0x10400000);
    let result = loader::load_ppu_elf(&data, &mut mem, &mut state).unwrap();

    // Entry descriptor at 0x846ae0 resolves to code at 0x10230, TOC 0x8969a8
    assert_eq!(result.entry, 0x846ae0);
    assert_eq!(state.pc, 0x10230);
    assert_eq!(state.gpr[2], 0x8969a8);

    // First instruction at PC should be nonzero (real code)
    let pc = state.pc as usize;
    let first_insn = u32::from_be_bytes([
        mem.as_bytes()[pc],
        mem.as_bytes()[pc + 1],
        mem.as_bytes()[pc + 2],
        mem.as_bytes()[pc + 3],
    ]);
    assert_ne!(
        first_insn, 0,
        "entry point should have code, got 0x00000000"
    );

    // Code segment should be populated (spot-check near entry)
    let code_region = &mem.as_bytes()[0x10000..0x10100];
    assert!(
        code_region.iter().any(|&b| b != 0),
        "code segment near base should contain nonzero bytes"
    );

    // Read-only data at 0x10000000 should be populated
    let rodata = &mem.as_bytes()[0x10000000..0x10000100];
    assert!(
        rodata.iter().any(|&b| b != 0),
        "read-only data segment should contain nonzero bytes"
    );
}

/// Boot progress regression: load a retail EBOOT, run the PPU, and
/// assert execution begins. Pins the current end-of-boot condition
/// for the wired test fixture so regressions surface as test
/// failures rather than silent behavior changes.
#[test]
fn retail_boot_progress() {
    use cellgov_core::{Runtime, StepError};

    let path =
        std::path::PathBuf::from("../../tools/rpcs3/dev_hdd0/game/NPUA80001/USRDIR/EBOOT.elf");
    if !path.exists() {
        return;
    }
    let data = std::fs::read(&path).unwrap();

    let required = loader::required_memory_size(&data).unwrap();
    let mem_size = ((required + 0xFFFF) & !0xFFFF) + 0x100000;
    let mut mem = GuestMemory::new(mem_size);

    // Plant exit stub at address 0
    let li_r11_22: u32 = (14 << 26) | (11 << 21) | 22;
    let sc: u32 = 0x4400_0002;
    let stub = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0), 8).unwrap();
    let mut sb = Vec::with_capacity(8);
    sb.extend_from_slice(&li_r11_22.to_be_bytes());
    sb.extend_from_slice(&sc.to_be_bytes());
    mem.apply_commit(stub, &sb).unwrap();

    let mut state = state::PpuState::new();
    loader::load_ppu_elf(&data, &mut mem, &mut state).unwrap();
    state.gpr[1] = (mem_size as u64) - 0x1000;
    state.lr = 0;

    let mut rt = Runtime::new(mem, Budget::new(100_000), 10_000);
    rt.registry_mut().register_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        *unit.state_mut() = state;
        unit
    });

    let mut steps = 0;
    let mut faulted = false;
    loop {
        match rt.step() {
            Ok(step) => {
                let _ = rt.commit_step(&step.result, &step.effects);
                steps += 1;
                if step.result.fault.is_some() {
                    faulted = true;
                    break;
                }
            }
            Err(StepError::NoRunnableUnit) => break,
            Err(StepError::MaxStepsExceeded) => break,
            Err(_) => break,
        }
    }

    // Gate: PPU must start executing (at least 1 step).
    assert!(
        steps >= 1,
        "PPU should execute at least 1 step, got {steps}"
    );

    // Current state: faults at step 1 with PC_OUT_OF_RANGE.
    // Update these assertions as the boot frontier advances.
    assert!(
        faulted,
        "expected fault (boot not yet complete), but PPU stalled after {steps} steps"
    );
}

#[test]
fn fault_includes_register_dump() {
    // Execute a PPU at PC=0 with no instructions in memory.
    // This faults immediately with PC_OUT_OF_RANGE and should
    // include a FaultRegisterDump with the GPR state.
    let mem = GuestMemory::new(64);
    let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
    ppu.state_mut().gpr[3] = 0xCAFE;
    ppu.state_mut().lr = 0x1000;
    ppu.state_mut().ctr = 0x2000;
    ppu.state_mut().cr = 0x80000000;
    ppu.state_mut().pc = 0x100; // past end of 64-byte memory

    let ctx = ExecutionContext::new(&mem);
    let result = ppu.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());

    assert_eq!(result.yield_reason, YieldReason::Fault);
    let regs = result
        .local_diagnostics
        .fault_regs
        .as_ref()
        .expect("fault should include register dump");
    assert_eq!(regs.gprs[3], 0xCAFE);
    assert_eq!(regs.lr, 0x1000);
    assert_eq!(regs.ctr, 0x2000);
    assert_eq!(regs.cr, 0x80000000);
}

#[test]
fn lha_sign_extends_negative_halfword() {
    // Place 0xFFFE (i16 = -2) at memory offset 8, then lha r3, 8(r0) at PC=0.
    let mut mem = GuestMemory::new(64);
    let neg_range = ByteRange::new(GuestAddr::new(8), 2).unwrap();
    mem.apply_commit(neg_range, &0xFFFEu16.to_be_bytes())
        .unwrap();
    // lha: opcode 42, RT=3, RA=0, D=8 -> (42<<26) | (3<<21) | 8
    let lha: u32 = (42 << 26) | (3 << 21) | 8;
    place_insn(&mut mem, 0, lha);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let _ = unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());

    // Sign-extended -2 as u64 = 0xFFFF_FFFF_FFFF_FFFE.
    assert_eq!(unit.state().gpr[3], 0xFFFF_FFFF_FFFF_FFFE);
}

#[test]
fn lha_zero_extends_positive_halfword() {
    // Place 0x1234 (positive) at offset 8, then lha r4, 8(r0) at PC=0.
    let mut mem = GuestMemory::new(64);
    let pos_range = ByteRange::new(GuestAddr::new(8), 2).unwrap();
    mem.apply_commit(pos_range, &0x1234u16.to_be_bytes())
        .unwrap();
    let lha: u32 = (42 << 26) | (4 << 21) | 8;
    place_insn(&mut mem, 0, lha);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let _ = unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());

    assert_eq!(unit.state().gpr[4], 0x1234);
}

/// Regression guard for the Lbzu update-form bug where the decoder
/// silently treated `lbzu` as `lbz` and skipped the RA writeback.
/// liblv2's strchr-style scan loop relies on `lbzu r0, 1(r9)` to
/// advance r9 by 1 each iteration; without the writeback, r9 stays
/// stuck and the loop spins forever. The test verifies BOTH that the
/// loaded byte matches what is actually at r9+1 AND that r9 has been
/// advanced to that address after the instruction retires.
#[test]
fn lbzu_advances_ra_to_effective_address() {
    let mut mem = GuestMemory::new(64);
    // Memory: at offset 0x10 store byte 0x2F (ASCII '/'), the byte
    // the actual liblv2 loop scans for.
    let target_addr: u64 = 0x10;
    let target_byte: u8 = 0x2F;
    let r = ByteRange::new(GuestAddr::new(target_addr), 1).unwrap();
    mem.apply_commit(r, &[target_byte]).unwrap();
    // lbzu r0, 1(r9): primary 35, RT=0, RA=9, D=1 -> 0x8C090001
    place_insn(&mut mem, 0, 0x8C09_0001);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    // r9 starts one byte BELOW the target; lbzu must advance it.
    unit.state_mut().gpr[9] = target_addr - 1;
    let ctx = ExecutionContext::new(&mem);
    let _ = unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());

    assert_eq!(
        unit.state().gpr[9],
        target_addr,
        "lbzu must update RA to the effective address",
    );
    assert_eq!(
        unit.state().gpr[0],
        target_byte as u64,
        "lbzu must load the byte at the effective address into RT",
    );
}

#[test]
fn per_step_trace_off_records_no_state_hashes() {
    // Per-step tracing off (default) means the unit produces an empty
    // drain even after retiring instructions. The hot-loop branch
    // must skip the push entirely.
    let mut mem = GuestMemory::new(64);
    place_insn(&mut mem, 0, (14 << 26) | (3 << 21) | 1); // addi r3, r0, 1
    place_insn(&mut mem, 4, (14 << 26) | (4 << 21) | 2); // addi r4, r0, 2
    place_insn(&mut mem, 8, (14 << 26) | (5 << 21) | 3); // addi r5, r0, 3

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    use cellgov_exec::ExecutionUnit;
    let ctx = ExecutionContext::new(&mem);
    let _ = unit.run_until_yield(Budget::new(3), &ctx, &mut Vec::new());

    let drained = unit.drain_retired_state_hashes();
    assert!(
        drained.is_empty(),
        "per_step_trace off must produce no fingerprints, got {} entries",
        drained.len()
    );
}

#[test]
fn per_step_trace_on_records_one_hash_per_retired_instruction() {
    use cellgov_exec::ExecutionUnit;
    let mut mem = GuestMemory::new(64);
    place_insn(&mut mem, 0, (14 << 26) | (3 << 21) | 1); // addi r3, r0, 1
    place_insn(&mut mem, 4, (14 << 26) | (4 << 21) | 2); // addi r4, r0, 2
    place_insn(&mut mem, 8, (14 << 26) | (5 << 21) | 3); // addi r5, r0, 3

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_per_step_trace(true);
    assert!(unit.per_step_trace());
    let ctx = ExecutionContext::new(&mem);
    let _ = unit.run_until_yield(Budget::new(3), &ctx, &mut Vec::new());

    let drained = unit.drain_retired_state_hashes();
    assert_eq!(drained.len(), 3, "one fingerprint per retired instruction");

    // PCs reflect the start of each retired instruction in order.
    assert_eq!(drained[0].0, 0);
    assert_eq!(drained[1].0, 4);
    assert_eq!(drained[2].0, 8);

    // State after each instruction differs: r3, r4, r5 are set in
    // turn. Adjacent hashes must differ.
    assert_ne!(drained[0].1, drained[1].1);
    assert_ne!(drained[1].1, drained[2].1);
}

#[test]
fn drain_retired_state_hashes_clears_buffer() {
    use cellgov_exec::ExecutionUnit;
    let mut mem = GuestMemory::new(64);
    place_insn(&mut mem, 0, (14 << 26) | (3 << 21) | 1);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_per_step_trace(true);
    let ctx = ExecutionContext::new(&mem);
    let _ = unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());

    assert_eq!(unit.drain_retired_state_hashes().len(), 1);
    assert_eq!(
        unit.drain_retired_state_hashes().len(),
        0,
        "drain must consume the buffer"
    );
}

#[test]
fn per_step_trace_does_not_record_on_fault() {
    // Decode-fault path: a bogus opcode (primary 0) cannot retire, so
    // no fingerprint is pushed even with per-step trace on.
    use cellgov_exec::ExecutionUnit;
    let mut mem = GuestMemory::new(64);
    place_insn(&mut mem, 0, 0x0000_0000);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_per_step_trace(true);
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());

    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert!(
        unit.drain_retired_state_hashes().is_empty(),
        "fault means the instruction did not retire; no fingerprint"
    );
}

#[test]
fn full_state_window_off_records_no_full_states() {
    // Default: window None -> drain returns empty even after
    // retiring instructions. Per-step hashes still produced when
    // their separate flag is on, so the two paths are independent.
    use cellgov_exec::ExecutionUnit;
    let mut mem = GuestMemory::new(64);
    place_insn(&mut mem, 0, (14 << 26) | (3 << 21) | 1);
    place_insn(&mut mem, 4, (14 << 26) | (4 << 21) | 2);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let _ = unit.run_until_yield(Budget::new(2), &ctx, &mut Vec::new());
    assert!(unit.drain_retired_state_full().is_empty());
}

#[test]
fn full_state_window_emits_only_inside_range() {
    // Retire 5 instructions; window is [1, 3] inclusive. Drain must
    // contain exactly indices 1..=3 with their PCs in order.
    use cellgov_exec::ExecutionUnit;
    let mut mem = GuestMemory::new(64);
    for i in 0..5 {
        place_insn(
            &mut mem,
            (i * 4) as usize,
            (14 << 26) | ((3 + i as u32 % 3) << 21) | (i as u32 + 1),
        );
    }

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_full_state_window(Some((1, 3)));
    assert_eq!(unit.full_state_window(), Some((1, 3)));
    let ctx = ExecutionContext::new(&mem);
    let _ = unit.run_until_yield(Budget::new(5), &ctx, &mut Vec::new());

    let drained = unit.drain_retired_state_full();
    assert_eq!(drained.len(), 3, "[1,3] inclusive == 3 entries");
    assert_eq!(drained[0].0, 4);
    assert_eq!(drained[1].0, 8);
    assert_eq!(drained[2].0, 12);
}

#[test]
fn full_state_window_and_per_step_hash_are_independent() {
    // With per-step trace ON and window [0, 0], the per-step stream
    // gets 5 entries, the window stream gets 1.
    use cellgov_exec::ExecutionUnit;
    let mut mem = GuestMemory::new(64);
    for i in 0..5 {
        place_insn(
            &mut mem,
            (i * 4) as usize,
            (14 << 26) | ((3 + i as u32 % 3) << 21) | (i as u32 + 1),
        );
    }

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_per_step_trace(true);
    unit.set_full_state_window(Some((0, 0)));
    let ctx = ExecutionContext::new(&mem);
    let _ = unit.run_until_yield(Budget::new(5), &ctx, &mut Vec::new());

    assert_eq!(unit.drain_retired_state_hashes().len(), 5);
    assert_eq!(unit.drain_retired_state_full().len(), 1);
}

#[test]
fn full_state_window_captures_actual_register_values() {
    // After retiring `addi r3, r0, 7`, GPR[3] must equal 7 in the
    // captured snapshot.
    use cellgov_exec::ExecutionUnit;
    let mut mem = GuestMemory::new(64);
    place_insn(&mut mem, 0, (14 << 26) | (3 << 21) | 7);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.set_full_state_window(Some((0, 0)));
    let ctx = ExecutionContext::new(&mem);
    let _ = unit.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());

    let drained = unit.drain_retired_state_full();
    assert_eq!(drained.len(), 1);
    let (_pc, gpr, _lr, _ctr, _xer, _cr) = drained[0];
    assert_eq!(gpr[3], 7);
}

#[test]
fn non_fault_step_has_no_register_dump() {
    // addi r3, r0, 42 at PC=0
    let raw: u32 = (14 << 26) | (3 << 21) | 42;
    let mut mem = GuestMemory::new(64);
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(0), 4).unwrap();
    mem.apply_commit(range, &raw.to_be_bytes()).unwrap();

    let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let result = ppu.run_until_yield(Budget::new(1), &ctx, &mut Vec::new());

    assert_eq!(result.yield_reason, YieldReason::BudgetExhausted);
    assert!(
        result.local_diagnostics.fault_regs.is_none(),
        "non-fault step should not include register dump"
    );
}

// -------------------------------------------------------------------
// Predecoded-shadow invalidation test suite.
//
// These tests establish the correctness contract for the
// predecoded instruction shadow. They must pass bit-identically
// whether the shadow is active or not.
//
// Pattern: each test seeds code that writes a result register to
// a scratch memory address (0x80) via `stw`, then reads committed
// memory to check the value. This avoids needing to access PPU
// register state through `dyn RegisteredUnit`.
// -------------------------------------------------------------------

mod insn_encode {
    pub fn li(rd: u32, simm: i16) -> u32 {
        (14 << 26) | (rd << 21) | ((simm as u16) as u32)
    }
    pub fn stw(rs: u32, ra: u32, d: i16) -> u32 {
        (36 << 26) | (rs << 21) | (ra << 16) | ((d as u16) as u32)
    }
    /// Relative unconditional branch. `offset` is a signed byte
    /// displacement from the current PC (must be 4-byte aligned).
    pub fn b(offset: i32) -> u32 {
        (18 << 26) | ((offset as u32) & 0x03FF_FFFC)
    }
}

fn step_n(rt: &mut cellgov_core::Runtime, n: usize) {
    for _ in 0..n {
        match rt.step() {
            Ok(s) => {
                let _ = rt.commit_step(&s.result, &s.effects);
            }
            Err(_) => break,
        }
    }
}

fn read_mem_u32(rt: &cellgov_core::Runtime, addr: usize) -> u32 {
    let m = rt.memory().as_bytes();
    u32::from_be_bytes([m[addr], m[addr + 1], m[addr + 2], m[addr + 3]])
}

/// Shadow invalidation scenario 1 -- CRT0 relocation replay.
///
/// Seed `li r3, 10` at PC=0, execute it, then overwrite PC=0 with
/// `li r3, 20` via apply_commit, loop back, execute again. The
/// store at 0x04 writes r3 to scratch address 0x80 each pass.
#[test]
fn shadow_inv_crt0_reloc_replay() {
    use insn_encode::*;
    let mut mem = GuestMemory::new(256);
    place_insn(&mut mem, 0x00, li(3, 10));
    place_insn(&mut mem, 0x04, stw(3, 0, 0x80));
    place_insn(&mut mem, 0x08, b(-8));

    let mut rt = cellgov_core::Runtime::new(mem, Budget::new(1), 100);
    rt.set_mode(cellgov_core::RuntimeMode::FaultDriven);
    rt.registry_mut().register_with(PpuExecutionUnit::new);

    // Pass 1 (3 steps): li r3,10 -> stw r3,0x80 -> b -8.
    step_n(&mut rt, 3);
    assert_eq!(read_mem_u32(&rt, 0x80), 10);

    // Overwrite PC=0 with `li r3, 20`.
    rt.memory_mut()
        .apply_commit(
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            &li(3, 20).to_be_bytes(),
        )
        .unwrap();

    // Pass 2 (3 steps): li r3,20 -> stw r3,0x80 -> b -8.
    step_n(&mut rt, 3);
    assert_eq!(
        read_mem_u32(&rt, 0x80),
        20,
        "rewritten instruction must be observed"
    );
}

/// Shadow invalidation scenario 2 -- HLE trampoline plant + execute + re-plant.
///
/// Plant `li r3, 111` at PC=0, execute, overwrite with
/// `li r3, 222`, loop back, execute again. Mimics the PRX binder
/// planting an HLE stub, executing it, then overwriting with a
/// different stub.
#[test]
fn shadow_inv_hle_trampoline_replant() {
    use insn_encode::*;
    let mut mem = GuestMemory::new(256);
    place_insn(&mut mem, 0x00, li(3, 111));
    place_insn(&mut mem, 0x04, stw(3, 0, 0x80));
    place_insn(&mut mem, 0x08, b(-8));

    let mut rt = cellgov_core::Runtime::new(mem, Budget::new(1), 100);
    rt.set_mode(cellgov_core::RuntimeMode::FaultDriven);
    rt.registry_mut().register_with(PpuExecutionUnit::new);

    step_n(&mut rt, 3);
    assert_eq!(read_mem_u32(&rt, 0x80), 111);

    // Re-plant with different value.
    rt.memory_mut()
        .apply_commit(
            ByteRange::new(GuestAddr::new(0), 4).unwrap(),
            &li(3, 222).to_be_bytes(),
        )
        .unwrap();

    step_n(&mut rt, 3);
    assert_eq!(
        read_mem_u32(&rt, 0x80),
        222,
        "second HLE stub value must be observed"
    );
}

/// Shadow invalidation scenario 3 -- Write-execute-rewrite-execute across commit
/// boundary.
///
/// Seed `li r3, 100`, execute + store to 0x80 (reads back 100),
/// then overwrite PC=0 with `li r3, 999`, execute again + store
/// (reads back 999). Both values must match the committed bytes at
/// their respective commit points.
#[test]
fn shadow_inv_write_exec_rewrite_exec() {
    use insn_encode::*;
    let mut mem = GuestMemory::new(256);
    place_insn(&mut mem, 0x00, li(3, 100));
    place_insn(&mut mem, 0x04, stw(3, 0, 0x80));
    place_insn(&mut mem, 0x08, b(-8));

    let mut rt = cellgov_core::Runtime::new(mem, Budget::new(1), 200);
    rt.set_mode(cellgov_core::RuntimeMode::FaultDriven);
    rt.registry_mut().register_with(PpuExecutionUnit::new);

    step_n(&mut rt, 3);
    assert_eq!(read_mem_u32(&rt, 0x80), 100, "first pass stores 100");

    rt.memory_mut()
        .apply_commit(
            ByteRange::new(GuestAddr::new(0x00), 4).unwrap(),
            &li(3, 999).to_be_bytes(),
        )
        .unwrap();

    step_n(&mut rt, 3);
    assert_eq!(
        read_mem_u32(&rt, 0x80),
        999,
        "second pass stores 999 after rewrite"
    );
}

/// Shadow invalidation scenario 4 -- Cross-slot write into text.
///
/// A 4-byte write that straddles two instruction-word-aligned
/// slots (offsets 2..6) must be reflected in both words on the
/// next fetch.
#[test]
fn shadow_inv_cross_slot_write() {
    use insn_encode::*;
    let mut mem = GuestMemory::new(256);
    // Seed: slot 0 = li r3, 0 (38 60 00 00), slot 1 = li r4, 0 (38 80 00 00).
    place_insn(&mut mem, 0x00, li(3, 0));
    place_insn(&mut mem, 0x04, li(4, 0));
    place_insn(&mut mem, 0x08, stw(3, 0, 0x80));
    place_insn(&mut mem, 0x0C, b(0));

    // Patch bytes 2..6: [00 0A 38 C0]
    //   slot 0 becomes: 38 60 00 0A = li r3, 10
    //   slot 1 becomes: 38 C0 00 00 = li r6, 0 (changed rD)
    let patch = [0x00u8, 0x0A, 0x38, 0xC0];
    mem.apply_commit(ByteRange::new(GuestAddr::new(2), 4).unwrap(), &patch)
        .unwrap();

    let mut rt = cellgov_core::Runtime::new(mem, Budget::new(1), 50);
    rt.set_mode(cellgov_core::RuntimeMode::FaultDriven);
    rt.registry_mut().register_with(PpuExecutionUnit::new);

    // li r3,10 -> li r6,0 -> stw r3,0x80 -> spin.
    step_n(&mut rt, 3);
    assert_eq!(
        read_mem_u32(&rt, 0x80),
        10,
        "slot 0 must reflect the cross-slot patch"
    );
}

/// Shadow invalidation scenario 5 -- Partial-word write into a text slot.
///
/// A single-byte write into the low byte of an instruction word
/// must produce the partially-modified instruction on the next
/// fetch.
#[test]
fn shadow_inv_partial_word_write() {
    use insn_encode::*;
    let mut mem = GuestMemory::new(256);
    // li r3, 0 at 0x00: bytes 38 60 00 00.
    place_insn(&mut mem, 0x00, li(3, 0));
    place_insn(&mut mem, 0x04, stw(3, 0, 0x80));
    place_insn(&mut mem, 0x08, b(0));

    // Patch byte 3 from 0x00 to 0x2A -> li r3, 42.
    mem.apply_commit(ByteRange::new(GuestAddr::new(3), 1).unwrap(), &[0x2A])
        .unwrap();

    let mut rt = cellgov_core::Runtime::new(mem, Budget::new(1), 50);
    rt.set_mode(cellgov_core::RuntimeMode::FaultDriven);
    rt.registry_mut().register_with(PpuExecutionUnit::new);

    // li r3,42 -> stw r3,0x80.
    step_n(&mut rt, 2);
    assert_eq!(
        read_mem_u32(&rt, 0x80),
        42,
        "partial-byte patch must be observed"
    );
}

// -- Mid-block fault recovery --

#[test]
fn mid_block_fault_rolls_back_and_propagates_directly() {
    // Two instructions: addi r3, r3, 1 (succeeds), then lwz from a
    // bad address (faults). At Budget>1 the addi retires locally;
    // when lwz faults the fault rule discards every effect from the
    // batch, so state must roll back to the snapshot for determinism.
    // The fault then propagates directly in one yield -- the
    // diagnostic reports the actual faulting PC even though state
    // is rolled back to the batch start.
    let mut mem = GuestMemory::new(256);
    let addi_r3: u32 = (14 << 26) | (3 << 21) | (3 << 16) | 1; // addi r3,r3,1
    let lwz_bad: u32 = (32 << 26) | (4 << 21) | (5 << 16); // lwz r4, 0(r5)
    place_insn(&mut mem, 0, addi_r3);
    place_insn(&mut mem, 4, lwz_bad);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().gpr[3] = 10;
    unit.state_mut().gpr[5] = 0xFFFF_0000; // bad address

    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);

    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert!(result.fault.is_some());
    assert_eq!(unit.status(), UnitStatus::Faulted);
    assert_eq!(
        unit.state().gpr[3],
        10,
        "GPR[3] rolled back to pre-block value (fault rule discards all effects)"
    );
    assert_eq!(unit.state().pc, 0, "PC rolled back to block start");
    assert!(effects.is_empty(), "no effects committed");
    assert_eq!(
        result.local_diagnostics.pc,
        Some(4),
        "diagnostic reports the actual faulting PC, not the batch start"
    );
}

#[test]
fn first_instruction_fault_reports_directly_at_budget_gt_1() {
    // When the very first instruction in a budget>1 step faults,
    // no snapshot restore is needed -- fault is reported directly.
    let mut mem = GuestMemory::new(256);
    let lwz_bad: u32 = (32 << 26) | (4 << 21) | (5 << 16);
    place_insn(&mut mem, 0, lwz_bad);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().gpr[5] = 0xFFFF_0000;

    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(64), &ctx, &mut effects);
    assert_eq!(r.yield_reason, YieldReason::Fault);
    assert_eq!(unit.status(), UnitStatus::Faulted);
}

#[test]
fn profile_mode_counts_raw_instructions() {
    let mut mem = GuestMemory::new(256);
    // addi r3, r0, 1 (li r3, 1)
    let addi: u32 = (14 << 26) | (3 << 21) | 1;
    // addi r4, r0, 2 (li r4, 2)
    let addi2: u32 = (14 << 26) | (4 << 21) | 2;
    // sc (syscall)
    let sc: u32 = 0x4400_0002;
    place_insn(&mut mem, 0, addi);
    place_insn(&mut mem, 4, addi2);
    place_insn(&mut mem, 8, sc);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().gpr[11] = 22; // sys_process_exit
    unit.set_profile_mode(true);

    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let r = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    assert_eq!(r.yield_reason, YieldReason::Syscall);

    let insns = unit.drain_profile_insns();
    assert!(insns
        .iter()
        .any(|(name, count)| *name == "Addi" && *count == 2));

    let pairs = unit.drain_profile_pairs();
    assert!(pairs
        .iter()
        .any(|((a, b), count)| *a == "Addi" && *b == "Addi" && *count == 1));
}

/// One PPU thread + one SPU unit contend on the same
/// 128-byte line. PPU runs a hand-encoded CAS retry loop for
/// PPU_N iterations; SPU runs the spu_atomic_cross_spu ELF for
/// SPU_N iterations. Under real cross-architecture reservation
/// contention the final counter is exactly `PPU_N + SPU_N`
/// regardless of scheduler interleaving. Cross-budget equivalence
/// is asserted by running the same scenario at Budget=1 and
/// Budget=256.
#[test]
fn cross_unit_atomic_conflict_ppu_vs_spu_counter_sums_cleanly() {
    use cellgov_spu::{loader as spu_loader, SpuExecutionUnit};
    use cellgov_testkit::fixtures::ScenarioFixture;

    const PPU_N: u32 = 16;
    const SPU_N: u32 = 32;

    let spu_elf_path =
        std::path::Path::new("../../tests/micro/spu_atomic_cross_spu/build/spu_main.elf");
    if !spu_elf_path.exists() {
        return;
    }
    let spu_elf = std::fs::read(spu_elf_path).unwrap();

    // Memory layout inside the single main-memory region.
    // 0x0000..0x0100:   reserved (avoid confusing null dereferences)
    // 0x0100..0x0120:   PPU program (8 instructions).
    // 0x1000..0x1080:   SPU result slot.
    // 0x10000..0x10080: atomic line (128-byte aligned).
    let ppu_pc: u64 = 0x100;
    let atomic_ea: u32 = 0x10000;
    let spu_result_ea: u32 = 0x1000;

    fn run_at_budget(
        spu_elf: Vec<u8>,
        ppu_pc: u64,
        atomic_ea: u32,
        spu_result_ea: u32,
        budget: u64,
    ) -> u32 {
        let fixture = ScenarioFixture::builder()
            .memory_size(0x2_0000)
            .budget(Budget::new(budget))
            // Cap generous enough for both units to finish N iterations
            // at the given budget and spin on their halt sentinels.
            .max_steps(20_000)
            .seed_memory(move |mem| {
                // PPU CAS loop. r3 = atomic_ea, r10 = PPU_N on entry.
                // 0x100: lwarx r9, 0, r3
                // 0x104: addi r9, r9, 1
                // 0x108: stwcx. r9, 0, r3
                // 0x10C: bne- cr0, 0x100  (retry CAS on failure)
                // 0x110: addi r10, r10, -1
                // 0x114: cmpwi cr7, r10, 0
                // 0x118: bne+ cr7, 0x100  (continue outer loop)
                // 0x11C: b 0x11C          (halt-spin)
                let program: [u32; 8] = [
                    0x7D20_1828, // lwarx r9, 0, r3
                    0x3929_0001, // addi  r9, r9, 1
                    0x7D20_192D, // stwcx. r9, 0, r3
                    0x4082_FFF4, // bne- cr0, -12
                    0x394A_FFFF, // addi r10, r10, -1
                    0x2F8A_0000, // cmpwi cr7, r10, 0
                    0x409E_FFE8, // bne+ cr7, -24
                    0x4800_0000, // b 0 (spin)
                ];
                let mut prog_bytes = Vec::with_capacity(32);
                for raw in program {
                    prog_bytes.extend_from_slice(&raw.to_be_bytes());
                }
                let range = ByteRange::new(GuestAddr::new(ppu_pc), 32).unwrap();
                mem.apply_commit(range, &prog_bytes).unwrap();
            })
            .register(move |rt| {
                rt.set_spu_factory(|id, init| {
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

                // Register PPU unit directly (no LV2 thread-group
                // plumbing needed for this scenario -- we drive the
                // scheduler at the unit level).
                rt.registry_mut().register_with(|id| {
                    let mut unit = PpuExecutionUnit::new(id);
                    unit.state_mut().pc = ppu_pc;
                    unit.state_mut().gpr[3] = atomic_ea as u64;
                    unit.state_mut().gpr[10] = PPU_N as u64;
                    unit
                });

                // Register SPU unit with the atomic ELF loaded.
                rt.registry_mut().register_with(|id| {
                    let mut unit = SpuExecutionUnit::new(id);
                    spu_loader::load_spu_elf(&spu_elf, unit.state_mut()).unwrap();
                    unit.state_mut().pc = 0x80;
                    unit.state_mut().set_reg_word_splat(1, 0x3FFF0);
                    unit.state_mut().set_reg_word_splat(3, 0xB);
                    unit.state_mut().set_reg_word_splat(4, atomic_ea);
                    unit.state_mut().set_reg_word_splat(5, spu_result_ea);
                    unit
                });
            })
            .build();

        let result = cellgov_testkit::run(fixture);
        let mem = &result.final_memory;
        let counter_bytes = &mem[atomic_ea as usize..atomic_ea as usize + 4];
        u32::from_be_bytes([
            counter_bytes[0],
            counter_bytes[1],
            counter_bytes[2],
            counter_bytes[3],
        ])
    }

    let expected = PPU_N + SPU_N;
    let counter_b1 = run_at_budget(spu_elf.clone(), ppu_pc, atomic_ea, spu_result_ea, 1);
    assert_eq!(
        counter_b1, expected,
        "cross-unit counter at Budget=1 must equal PPU_N + SPU_N"
    );
    let counter_b256 = run_at_budget(spu_elf, ppu_pc, atomic_ea, spu_result_ea, 256);
    assert_eq!(
        counter_b256, expected,
        "cross-unit counter at Budget=256 must equal PPU_N + SPU_N"
    );
}
