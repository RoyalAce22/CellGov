use super::*;
use cellgov_exec::ExecutionContext;
use cellgov_mem::GuestMemory;

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
    // li r11, 22 (sys_process_exit) then sc
    let mut mem = GuestMemory::new(256);
    // li r11, 22 = addi r11, r0, 22 = opcode 14, rt=11, ra=0, imm=22
    let li_r11_22: u32 = (14 << 26) | (11 << 21) | 22;
    place_insn(&mut mem, 0, li_r11_22);
    // sc = opcode 17, bit 1 set = 0x44000002
    place_insn(&mut mem, 4, 0x4400_0002);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(100), &ctx);
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
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(100), &ctx);
    assert_eq!(result.yield_reason, YieldReason::Finished);

    // Should have one SharedWriteIntent for the stw
    let writes: Vec<_> = result
        .emitted_effects
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
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(100), &ctx);
    assert_eq!(result.yield_reason, YieldReason::Finished);
    assert_eq!(unit.state().gpr[3], 0xDEAD_BEEF);
}

#[test]
fn unsupported_syscall_faults() {
    let mut mem = GuestMemory::new(256);
    // li r11, 999; sc
    let li: u32 = (14 << 26) | (11 << 21) | 999;
    place_insn(&mut mem, 0, li);
    place_insn(&mut mem, 4, 0x4400_0002);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(100), &ctx);
    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(unit.status(), UnitStatus::Faulted);
}

#[test]
fn decode_failure_faults() {
    let mut mem = GuestMemory::new(256);
    place_insn(&mut mem, 0, 0xFFFF_FFFF);

    let mut unit = PpuExecutionUnit::new(UnitId::new(0));
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(100), &ctx);
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
    let result = unit.run_until_yield(Budget::new(5), &ctx);
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
    let result = unit.run_until_yield(Budget::new(10), &ctx);
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
    let result = unit.run_until_yield(Budget::new(10), &ctx);
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
    let ctx = ExecutionContext::new(&mem);
    let budget = Budget::new(100_000);
    let result = unit.run_until_yield(budget, &ctx);
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

/// Build a ScenarioFixture that runs a real paired PPU+SPU
/// microtest. Both architecture units are pre-started by the
/// test harness: the SPU is registered with its ELF pre-loaded
/// into local store and its argp set to `result_ea`, and the
/// PPU is registered with its ELF pre-loaded into committed
/// guest memory, stack pointer near the top of memory, and LR
/// pointing at a planted exit stub at address 0. The SPU is not
/// launched through the PPU's LV2 `sys_spu_thread_group_start`
/// syscall (which is still a CELL_OK stub on the PPU side); it
/// is started directly, the same way an SPU-only scenario does.
fn build_paired_fixture(
    ppu_elf: Vec<u8>,
    spu_elf: Vec<u8>,
    spu_argps: Vec<u32>,
    budget: Budget,
    max_steps: usize,
    mailbox_value: Option<u32>,
) -> cellgov_testkit::fixtures::ScenarioFixture {
    use cellgov_spu::{loader as spu_loader, SpuExecutionUnit};
    use cellgov_testkit::fixtures::ScenarioFixture;
    use std::cell::RefCell;
    use std::rc::Rc;

    assert!(
        !spu_argps.is_empty(),
        "paired fixture needs at least one SPU argp"
    );

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
            // Plant exit stub at address 0 so a terminal `blr`
            // with LR = 0 lands on `li r11, 22; sc`.
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
            // Register the mailbox first when a pre-seeded value
            // is requested. The SPU's rdch(SPU_RdInMbox) handler
            // looks up MailboxId(unit_id), so the SPU MUST be the
            // unit whose id matches the mailbox. By registering
            // the mailbox before any unit, the mailbox gets id 0,
            // and the next unit we register (the SPU) also gets
            // id 0 -- the mapping the SPU-only microtest relies
            // on. The PPU then gets a later id, which has no
            // mailbox.
            if let Some(value) = mailbox_value {
                let mbox_id = rt.mailbox_registry_mut().register();
                assert_eq!(mbox_id.raw(), 0, "mailbox must be id 0 for the SPU");
                rt.mailbox_registry_mut()
                    .get_mut(mbox_id)
                    .unwrap()
                    .send(value);
            }

            let ppu_state = primed_reg.borrow_mut().take().unwrap();
            for argp in spu_argps {
                let elf_for_spu = spu_elf.clone();
                rt.registry_mut().register_with(move |id| {
                    let mut unit = SpuExecutionUnit::new(id);
                    spu_loader::load_spu_elf(&elf_for_spu, unit.state_mut()).unwrap();
                    unit.state_mut().pc = 0x80;
                    unit.state_mut().set_reg_word_splat(1, 0x3FFF0);
                    unit.state_mut().set_reg_word_splat(4, argp);
                    unit
                });
            }
            rt.registry_mut().register_with(|id| {
                let mut unit = PpuExecutionUnit::new(id);
                *unit.state_mut() = ppu_state;
                unit
            });
        })
        .build()
}

/// Drive a paired PPU+SPU microtest through the runner and
/// compare the resulting committed memory regions against the
/// settled RPCS3 baselines for this test. Silently skips when
/// either ELF or either baseline is missing.
fn run_paired_microtest_against_baseline(
    microtest: &str,
    spu_argps: Vec<u32>,
    budget: Budget,
    max_steps: usize,
    regions: Vec<cellgov_compare::RegionDescriptor>,
    mailbox_value: Option<u32>,
) {
    let ppu_path_buf = std::path::PathBuf::from(format!(
        "../../tests/micro/{}/build/{}.elf",
        microtest, microtest
    ));
    let spu_path_buf = std::path::PathBuf::from(format!(
        "../../tests/micro/{}/build/spu_main.elf",
        microtest
    ));
    let baseline_dir = std::path::PathBuf::from(format!("../../baselines/{}", microtest));
    let interp_path = baseline_dir.join("rpcs3_interpreter.json");
    let llvm_path = baseline_dir.join("rpcs3_llvm.json");
    if !ppu_path_buf.exists()
        || !spu_path_buf.exists()
        || !interp_path.exists()
        || !llvm_path.exists()
    {
        return;
    }
    let ppu_elf = std::fs::read(&ppu_path_buf).unwrap();
    let spu_elf = std::fs::read(&spu_path_buf).unwrap();

    let factory = move || {
        build_paired_fixture(
            ppu_elf.clone(),
            spu_elf.clone(),
            spu_argps.clone(),
            budget,
            max_steps,
            mailbox_value,
        )
    };

    let cellgov_obs = cellgov_compare::observe_with_determinism_check(factory, &regions).unwrap();
    assert_eq!(
        cellgov_obs.outcome,
        cellgov_compare::ObservedOutcome::Completed,
        "{} paired scenario did not complete",
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
        "{} paired scenario diverges from RPCS3: {:?}",
        microtest,
        result.cellgov_result
    );
}

#[test]
fn spu_fixed_value_paired_ppu_spu_matches_rpcs3_baseline() {
    let result_ea: u64 = 0x1_0000;
    let regions = vec![cellgov_compare::RegionDescriptor {
        name: "result".into(),
        addr: result_ea,
        size: 8,
    }];
    run_paired_microtest_against_baseline(
        "spu_fixed_value",
        vec![result_ea as u32],
        Budget::new(10_000),
        1_000,
        regions,
        None,
    );
}

#[test]
fn dma_completion_paired_ppu_spu_matches_rpcs3_baseline() {
    let result_ea: u64 = 0x1_0000;
    let regions = vec![
        cellgov_compare::RegionDescriptor {
            name: "header".into(),
            addr: result_ea,
            size: 8,
        },
        cellgov_compare::RegionDescriptor {
            name: "pattern".into(),
            addr: result_ea + 16,
            size: 128,
        },
    ];
    run_paired_microtest_against_baseline(
        "dma_completion",
        vec![result_ea as u32],
        Budget::new(100_000),
        1_000,
        regions,
        None,
    );
}

#[test]
fn ls_to_shared_paired_ppu_spu_matches_rpcs3_baseline() {
    let result_ea: u64 = 0x1_0000;
    let regions = vec![
        cellgov_compare::RegionDescriptor {
            name: "header".into(),
            addr: result_ea,
            size: 8,
        },
        cellgov_compare::RegionDescriptor {
            name: "data".into(),
            addr: result_ea + 16,
            size: 128,
        },
    ];
    run_paired_microtest_against_baseline(
        "ls_to_shared",
        vec![result_ea as u32],
        Budget::new(100_000),
        1_000,
        regions,
        None,
    );
}

#[test]
fn atomic_reservation_paired_ppu_spu_matches_rpcs3_baseline() {
    // atomic_reservation drives the SPU through mfc_getllar,
    // local-store updates, and mfc_putllc with a matching
    // reservation, then a final mfc_put to publish the result.
    // The paired PPU runs its CRT0 alongside (LV2 calls still
    // stubbed). Observable regions match the SPU-only test:
    // 8-byte header at EA+0 and 128-byte data at EA+16.
    let result_ea: u64 = 0x1_0000;
    let regions = vec![
        cellgov_compare::RegionDescriptor {
            name: "header".into(),
            addr: result_ea,
            size: 8,
        },
        cellgov_compare::RegionDescriptor {
            name: "data".into(),
            addr: result_ea + 16,
            size: 128,
        },
    ];
    run_paired_microtest_against_baseline(
        "atomic_reservation",
        vec![result_ea as u32],
        Budget::new(100_000),
        1_000,
        regions,
        None,
    );
}

#[test]
fn mailbox_roundtrip_paired_ppu_spu_matches_rpcs3_baseline() {
    // mailbox_roundtrip requires a pre-seeded SPU inbound
    // mailbox: the PPU sends 0x42, the SPU reads it, XORs with
    // 0xFFFFFFFF -> 0xFFFFFFBD, and writes the result back via
    // DMA put. In this paired scenario the SPU is still
    // pre-started by the harness (LV2 stub), so the mailbox
    // value is seeded directly into the mailbox registry before
    // the units are registered.
    let result_ea: u64 = 0x1_0000;
    let regions = vec![cellgov_compare::RegionDescriptor {
        name: "result".into(),
        addr: result_ea,
        size: 8,
    }];
    run_paired_microtest_against_baseline(
        "mailbox_roundtrip",
        vec![result_ea as u32],
        Budget::new(100_000),
        1_000,
        regions,
        Some(0x42),
    );
}

#[test]
fn barrier_wakeup_paired_ppu_spu_matches_rpcs3_baseline() {
    // barrier_wakeup is the only microtest with two SPU
    // threads. Both SPUs run the same ELF; they distinguish
    // themselves by the low bit of their argp. SPU 0 gets
    // base_ea | 0 and SPU 1 gets base_ea | 1; each writes its
    // result into a 16-byte-aligned slot in the shared buffer.
    // The PPU runs its CRT0 alongside both SPUs through a
    // single runtime scheduler.
    let base_ea: u64 = 0x1_0000;
    let regions = vec![
        cellgov_compare::RegionDescriptor {
            name: "spu0_result".into(),
            addr: base_ea,
            size: 8,
        },
        cellgov_compare::RegionDescriptor {
            name: "spu1_result".into(),
            addr: base_ea + 16,
            size: 8,
        },
    ];
    run_paired_microtest_against_baseline(
        "barrier_wakeup",
        vec![base_ea as u32, (base_ea | 1) as u32],
        Budget::new(10_000),
        100_000,
        regions,
        None,
    );
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
