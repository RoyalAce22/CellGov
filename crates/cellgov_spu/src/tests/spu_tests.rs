use super::*;
use cellgov_exec::ExecutionContext;
use cellgov_mem::GuestMemory;

#[test]
fn new_unit_is_runnable() {
    let unit = SpuExecutionUnit::new(UnitId::new(0));
    assert_eq!(unit.status(), UnitStatus::Runnable);
    assert_eq!(unit.unit_id(), UnitId::new(0));
}

#[test]
fn stop_instruction_yields_finished() {
    let mut unit = SpuExecutionUnit::new(UnitId::new(1));
    // Place `stop 0` (0x00000000) at PC=0
    // (LS is already zeroed, and 0x00000000 decodes to stop)
    let mem = GuestMemory::new(16);
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::Finished);
    assert_eq!(unit.status(), UnitStatus::Finished);
}

#[test]
fn il_then_stop_executes_two_instructions() {
    let mut unit = SpuExecutionUnit::new(UnitId::new(2));
    // il $3, 42 at PC=0, stop 0 at PC=4
    // il $3, 42: op9=0x081, rt=3, imm=42
    // Encoding: 0x081 << 23 | 42 << 7 | 3 = 0x40800D43
    let il_raw: u32 = 0x081 << 23 | (42u32 << 7) | 3;
    let il_bytes = il_raw.to_be_bytes();
    unit.state_mut().ls[0..4].copy_from_slice(&il_bytes);
    // stop 0 at PC=4 is already 0x00000000

    let mem = GuestMemory::new(16);
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::Finished);
    assert_eq!(unit.state().reg_word(3), 42);
}

#[test]
fn budget_exhaustion_yields() {
    let mut unit = SpuExecutionUnit::new(UnitId::new(3));
    // Place nop (0x40200000) at every 4-byte boundary
    let nop_bytes = 0x4020_0000u32.to_be_bytes();
    for i in (0..256).step_by(4) {
        unit.state_mut().ls[i..i + 4].copy_from_slice(&nop_bytes);
    }

    let mem = GuestMemory::new(16);
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(5), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::BudgetExhausted);
    assert_eq!(result.consumed_budget, Budget::new(5));
    assert_eq!(unit.state().pc, 20); // 5 nops * 4 bytes
}

#[test]
fn decode_failure_faults() {
    let mut unit = SpuExecutionUnit::new(UnitId::new(4));
    // Place an invalid instruction
    let bad = 0xFFFF_FFFFu32.to_be_bytes();
    unit.state_mut().ls[0..4].copy_from_slice(&bad);

    let mem = GuestMemory::new(16);
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(unit.status(), UnitStatus::Faulted);
    assert!(result.fault.is_some());
}

#[test]
fn lqa_out_of_range_local_store_faults() {
    // The ls_addr helper guards against a local-store access that
    // would overflow the current LS buffer. Under normal conditions
    // (256KB LS + 18-bit masked address) this cannot fire -- the
    // mask always produces a value within bounds. Verify the
    // defensive path works by truncating the LS so the masked
    // address is now past the end, and issuing an Lqa that the
    // ls_addr helper must reject.
    //
    // lqa rt=3, imm = 0x7FFE: effective LS offset = 0x7FFE << 2 = 0x1FFF8
    // which, masked to 0x3FFF0, gives 0x1FFF0. With LS truncated to
    // 0x1_0000 bytes, 0x1FFF0 + 16 > 0x1_0000 and the helper faults.
    let raw = (0x061u32 << 23) | 3 | ((0x7FFEu32 & 0xFFFF) << 7);
    let mut unit = SpuExecutionUnit::new(UnitId::new(0));
    unit.state_mut().ls.truncate(0x1_0000);
    unit.state_mut().ls[0..4].copy_from_slice(&raw.to_be_bytes());

    let mem = GuestMemory::new(16);
    let ctx = ExecutionContext::new(&mem);
    let result = unit.run_until_yield(Budget::new(10), &ctx, &mut Vec::new());
    assert_eq!(result.yield_reason, YieldReason::Fault);
    assert_eq!(unit.status(), UnitStatus::Faulted);
    // The fault must be a guest-side LS out-of-range encoded with
    // the FAULT_LS_OUT_OF_RANGE marker in the high bits.
    if let Some(FaultKind::Guest(code)) = result.fault {
        assert_eq!(code & FAULT_LS_OUT_OF_RANGE, FAULT_LS_OUT_OF_RANGE);
    } else {
        panic!(
            "expected Guest(FAULT_LS_OUT_OF_RANGE) fault, got {:?}",
            result.fault
        );
    }
}

#[test]
fn snapshot_captures_state() {
    let mut unit = SpuExecutionUnit::new(UnitId::new(5));
    unit.state_mut().set_reg_word_splat(7, 0xBEEF);
    unit.state_mut().pc = 0x100;
    let snap = unit.snapshot();
    assert_eq!(snap.pc, 0x100);
    assert_eq!(
        u32::from_be_bytes([
            snap.regs[7][0],
            snap.regs[7][1],
            snap.regs[7][2],
            snap.regs[7][3]
        ]),
        0xBEEF
    );
}

#[test]
fn wrch_mfc_cmd_yields_dma_submitted() {
    let mut unit = SpuExecutionUnit::new(UnitId::new(6));
    let s = unit.state_mut();

    // Build MFC DMA put command in LS as a sequence of wrch instructions.
    // We'll set up channel state directly and just place wrch MFC_CMD.
    s.channels.mfc_lsa = 0x3000;
    s.channels.mfc_eal = 0x10000;
    s.channels.mfc_size = 16;
    s.channels.mfc_tag_id = 0;

    // il $10, 0x20 (MFC_PUT) at PC=0
    let il_raw: u32 = 0x081 << 23 | (0x20u32 << 7) | 10;
    s.ls[0..4].copy_from_slice(&il_raw.to_be_bytes());
    // wrch $ch21, $10 at PC=4 (MFC_CMD = 21)
    // wrch encoding: op11=0x10D, ra=channel(21), rt=10
    let wrch_raw: u32 = 0x10D << 21 | (21u32 << 7) | 10;
    s.ls[4..8].copy_from_slice(&wrch_raw.to_be_bytes());

    let mem = GuestMemory::new(16);
    let ctx = ExecutionContext::new(&mem);
    let mut effects = Vec::new();
    let result = unit.run_until_yield(Budget::new(100), &ctx, &mut effects);
    assert_eq!(result.yield_reason, YieldReason::DmaSubmitted);
    assert_eq!(effects.len(), 1);
    assert!(matches!(
        &effects[0],
        cellgov_effects::Effect::DmaEnqueue {
            payload: Some(_),
            ..
        }
    ));
}

#[test]
fn run_spu_fixed_value_binary() {
    let path = std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_main.elf");
    if !path.exists() {
        return; // skip if not built
    }
    let elf_data = std::fs::read(path).unwrap();

    let mut unit = SpuExecutionUnit::new(UnitId::new(10));
    loader::load_spu_elf(&elf_data, unit.state_mut()).unwrap();

    // Bypass the C runtime -- jump directly to main() at 0x80.
    // Set up stack pointer ($1) and ABI registers as _start would.
    // main(speid=$3, argp=$4, envp=$5) -- argp is the result EA.
    unit.state_mut().pc = 0x80;
    unit.state_mut().set_reg_word_splat(1, 0x3FFF0); // stack
    let result_ea: u32 = 0x1_0000;
    unit.state_mut().set_reg_word_splat(4, result_ea);

    let mem = GuestMemory::new(0x2_0000);
    let ctx = ExecutionContext::new(&mem);

    // Run until the SPU finishes or faults. Collect all DMA effects.
    let mut all_effects = Vec::new();
    let max_steps = 50;
    for _ in 0..max_steps {
        let mut step_effects = Vec::new();
        let result = unit.run_until_yield(Budget::new(10000), &ctx, &mut step_effects);
        let reason = result.yield_reason;
        let fault = result.fault;
        all_effects.extend(step_effects);

        match reason {
            YieldReason::Finished => break,
            YieldReason::DmaSubmitted => continue,
            YieldReason::BudgetExhausted => continue,
            YieldReason::Fault => {
                panic!(
                    "SPU faulted at PC=0x{:04X}, fault={:?}",
                    unit.state().pc,
                    fault
                );
            }
            other => panic!(
                "unexpected yield {:?} at PC=0x{:04X}",
                other,
                unit.state().pc
            ),
        }
    }

    assert_eq!(unit.status(), UnitStatus::Finished);

    // The SPU should have emitted at least one DmaEnqueue effect
    // (the mfc_put of the result buffer).
    let dma_count = all_effects
        .iter()
        .filter(|e| matches!(e, cellgov_effects::Effect::DmaEnqueue { .. }))
        .count();
    assert!(
        dma_count >= 1,
        "expected at least 1 DMA put, got {}",
        dma_count
    );

    // The DMA put carries LS bytes as an inline payload. The first
    // 8 bytes must match the RPCS3 baseline (the compiled binary
    // may DMA more than 8 bytes due to SPU alignment rounding).
    let dma = all_effects
        .iter()
        .find(|e| matches!(e, cellgov_effects::Effect::DmaEnqueue { .. }))
        .expect("expected DmaEnqueue");
    if let cellgov_effects::Effect::DmaEnqueue {
        request, payload, ..
    } = dma
    {
        assert_eq!(request.destination().start().raw(), result_ea as u64);
        let data = payload.as_ref().expect("DMA put should carry payload");
        // RPCS3 baseline: [0, 0, 0, 0, 19, 55, 186, 173]
        // = 0x00000000 (status) + 0x1337BAAD (value)
        assert_eq!(
            &data[..8],
            &[0x00, 0x00, 0x00, 0x00, 0x13, 0x37, 0xBA, 0xAD],
            "DMA payload does not match RPCS3 baseline"
        );
    }
}

#[test]
fn mailbox_roundtrip_matches_rpcs3_baseline() {
    let elf_path = std::path::Path::new("../../tests/micro/mailbox_roundtrip/build/spu_main.elf");
    if !elf_path.exists() {
        return;
    }

    let baseline_dir = std::path::Path::new("../../baselines/mailbox_roundtrip");
    let interp_path = baseline_dir.join("rpcs3_interpreter.json");
    let llvm_path = baseline_dir.join("rpcs3_llvm.json");
    if !interp_path.exists() || !llvm_path.exists() {
        return;
    }

    let elf_data = std::fs::read(elf_path).unwrap();
    let result_ea: u64 = 0x1_0000;
    // PPU sends 0x42 to SPU inbound mailbox. SPU XORs with
    // 0xFFFFFFFF -> 0xFFFFFFBD. Baseline: [0,0,0,0, 255,255,255,189].
    let mailbox_value: u32 = 0x42;

    let factory = || {
        let elf = elf_data.clone();
        cellgov_testkit::fixtures::ScenarioFixture::builder()
            .memory_size(0x2_0000)
            .budget(Budget::new(10_000))
            .max_steps(1_000)
            .register(move |rt| {
                // Register the mailbox first (gets ID 0), then the
                // unit (also gets ID 0). The SPU's rdch handler
                // looks up MailboxId(unit_id), so the IDs must match.
                let mbox_id = rt.mailbox_registry_mut().register();
                rt.mailbox_registry_mut()
                    .get_mut(mbox_id)
                    .unwrap()
                    .send(mailbox_value);

                let data = elf;
                rt.registry_mut().register_with(|id| {
                    assert_eq!(id.raw(), mbox_id.raw(), "mailbox/unit ID mismatch");
                    let mut unit = SpuExecutionUnit::new(id);
                    loader::load_spu_elf(&data, unit.state_mut()).unwrap();
                    unit.state_mut().pc = 0x80;
                    unit.state_mut().set_reg_word_splat(1, 0x3FFF0);
                    unit.state_mut().set_reg_word_splat(4, result_ea as u32);
                    unit
                });
            })
            .build()
    };

    let regions = vec![cellgov_compare::RegionDescriptor {
        name: "result".into(),
        addr: result_ea,
        size: 8,
    }];

    let cellgov_obs = cellgov_compare::observe_with_determinism_check(factory, &regions).unwrap();
    assert_eq!(
        cellgov_obs.outcome,
        cellgov_compare::ObservedOutcome::Completed
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
        "mailbox_roundtrip diverges from RPCS3: {:?}",
        result.cellgov_result
    );
}

#[test]
fn atomic_reservation_matches_rpcs3_baseline() {
    let elf_path = std::path::Path::new("../../tests/micro/atomic_reservation/build/spu_main.elf");
    if !elf_path.exists() {
        return;
    }

    let baseline_dir = std::path::Path::new("../../baselines/atomic_reservation");
    let interp_path = baseline_dir.join("rpcs3_interpreter.json");
    let llvm_path = baseline_dir.join("rpcs3_llvm.json");
    if !interp_path.exists() || !llvm_path.exists() {
        return;
    }

    let elf_data = std::fs::read(elf_path).unwrap();
    let result_ea: u64 = 0x1_0000;

    let factory = || {
        let elf = elf_data.clone();
        cellgov_testkit::fixtures::ScenarioFixture::builder()
            .memory_size(0x2_0000)
            .budget(Budget::new(100_000))
            .max_steps(1_000)
            .register(move |rt| {
                let data = elf;
                rt.registry_mut().register_with(|id| {
                    let mut unit = SpuExecutionUnit::new(id);
                    loader::load_spu_elf(&data, unit.state_mut()).unwrap();
                    unit.state_mut().pc = 0x80;
                    unit.state_mut().set_reg_word_splat(1, 0x3FFF0);
                    unit.state_mut().set_reg_word_splat(4, result_ea as u32);
                    unit
                });
            })
            .build()
    };

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

    let cellgov_obs = cellgov_compare::observe_with_determinism_check(factory, &regions).unwrap();
    assert_eq!(
        cellgov_obs.outcome,
        cellgov_compare::ObservedOutcome::Completed
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
        "atomic_reservation diverges from RPCS3: {:?}",
        result.cellgov_result
    );
}

#[test]
fn barrier_wakeup_matches_rpcs3_baseline() {
    let elf_path = std::path::Path::new("../../tests/micro/barrier_wakeup/build/spu_main.elf");
    if !elf_path.exists() {
        return;
    }

    let baseline_dir = std::path::Path::new("../../baselines/barrier_wakeup");
    let interp_path = baseline_dir.join("rpcs3_interpreter.json");
    let llvm_path = baseline_dir.join("rpcs3_llvm.json");
    if !interp_path.exists() || !llvm_path.exists() {
        return;
    }

    let elf_data = std::fs::read(elf_path).unwrap();
    // Buffer is 256-byte aligned. Low byte encodes thread index.
    let base_ea: u64 = 0x1_0000;

    let factory = || {
        let elf = elf_data.clone();
        cellgov_testkit::fixtures::ScenarioFixture::builder()
            .memory_size(0x2_0000)
            .budget(Budget::new(10_000))
            .max_steps(100_000)
            .register(move |rt| {
                // SPU 0: argp = base_ea | 0
                let elf0 = elf.clone();
                rt.registry_mut().register_with(|id| {
                    let mut unit = SpuExecutionUnit::new(id);
                    loader::load_spu_elf(&elf0, unit.state_mut()).unwrap();
                    unit.state_mut().pc = 0x80;
                    unit.state_mut().set_reg_word_splat(1, 0x3FFF0);
                    unit.state_mut().set_reg_word_splat(4, base_ea as u32);
                    unit
                });

                // SPU 1: argp = base_ea | 1
                let elf1 = elf;
                rt.registry_mut().register_with(|id| {
                    let mut unit = SpuExecutionUnit::new(id);
                    loader::load_spu_elf(&elf1, unit.state_mut()).unwrap();
                    unit.state_mut().pc = 0x80;
                    unit.state_mut().set_reg_word_splat(1, 0x3FFF0);
                    unit.state_mut().set_reg_word_splat(4, (base_ea | 1) as u32);
                    unit
                });
            })
            .build()
    };

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

    let cellgov_obs = cellgov_compare::observe_with_determinism_check(factory, &regions).unwrap();
    assert_eq!(
        cellgov_obs.outcome,
        cellgov_compare::ObservedOutcome::Completed
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
        "barrier_wakeup diverges from RPCS3: {:?}",
        result.cellgov_result
    );
}

#[test]
fn ls_to_shared_matches_rpcs3_baseline() {
    let elf_path = std::path::Path::new("../../tests/micro/ls_to_shared/build/spu_main.elf");
    if !elf_path.exists() {
        return;
    }

    let baseline_dir = std::path::Path::new("../../baselines/ls_to_shared");
    let interp_path = baseline_dir.join("rpcs3_interpreter.json");
    let llvm_path = baseline_dir.join("rpcs3_llvm.json");
    if !interp_path.exists() || !llvm_path.exists() {
        return;
    }

    let elf_data = std::fs::read(elf_path).unwrap();
    let result_ea: u64 = 0x1_0000;

    let factory = || {
        let elf = elf_data.clone();
        cellgov_testkit::fixtures::ScenarioFixture::builder()
            .memory_size(0x2_0000)
            .budget(Budget::new(100_000))
            .max_steps(1_000)
            .register(move |rt| {
                let data = elf;
                rt.registry_mut().register_with(|id| {
                    let mut unit = SpuExecutionUnit::new(id);
                    loader::load_spu_elf(&data, unit.state_mut()).unwrap();
                    unit.state_mut().pc = 0x80;
                    unit.state_mut().set_reg_word_splat(1, 0x3FFF0);
                    unit.state_mut().set_reg_word_splat(4, result_ea as u32);
                    unit
                });
            })
            .build()
    };

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

    let cellgov_obs = cellgov_compare::observe_with_determinism_check(factory, &regions).unwrap();
    assert_eq!(
        cellgov_obs.outcome,
        cellgov_compare::ObservedOutcome::Completed
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
        "ls_to_shared diverges from RPCS3: {:?}",
        result.cellgov_result
    );
}

#[test]
fn dma_completion_payloads_are_correct() {
    let path = std::path::Path::new("../../tests/micro/dma_completion/build/spu_main.elf");
    if !path.exists() {
        return;
    }
    let elf_data = std::fs::read(path).unwrap();

    let mut unit = SpuExecutionUnit::new(UnitId::new(20));
    loader::load_spu_elf(&elf_data, unit.state_mut()).unwrap();
    unit.state_mut().pc = 0x80;
    unit.state_mut().set_reg_word_splat(1, 0x3FFF0);
    unit.state_mut().set_reg_word_splat(4, 0x1_0000u32);

    let mem = GuestMemory::new(0x2_0000);
    let ctx = ExecutionContext::new(&mem);

    let mut dma_payloads = Vec::new();
    for _ in 0..100 {
        let mut step_effects = Vec::new();
        let result = unit.run_until_yield(Budget::new(100_000), &ctx, &mut step_effects);
        for e in &step_effects {
            if let cellgov_effects::Effect::DmaEnqueue {
                request, payload, ..
            } = e
            {
                dma_payloads.push((
                    request.destination().start().raw(),
                    request.length(),
                    payload.clone(),
                ));
            }
        }
        match result.yield_reason {
            YieldReason::Finished => break,
            YieldReason::DmaSubmitted | YieldReason::BudgetExhausted => continue,
            other => panic!("unexpected: {:?}", other),
        }
    }

    assert_eq!(unit.status(), UnitStatus::Finished);
    assert_eq!(dma_payloads.len(), 2, "expected 2 DMA puts");

    // DMA #1: 128 bytes of [DE AD BE EF] pattern to EA+16
    let (dest1, len1, pay1) = &dma_payloads[0];
    assert_eq!(*dest1, 0x1_0010); // EA + 16
    assert_eq!(*len1, 128);
    let data1 = pay1.as_ref().unwrap();
    assert_eq!(data1[0..4], [0xDE, 0xAD, 0xBE, 0xEF]);

    // DMA #2: 16 bytes status header to EA
    let (dest2, len2, pay2) = &dma_payloads[1];
    assert_eq!(*dest2, 0x1_0000); // EA
    assert_eq!(*len2, 16);
    let data2 = pay2.as_ref().unwrap();
    // [status=0, pattern_size=128, 0, 0] as big-endian u32s
    assert_eq!(
        &data2[..8],
        &[0, 0, 0, 0, 0, 0, 0, 128],
        "header payload mismatch: {:?}",
        data2
    );
}

#[test]
fn dma_completion_matches_rpcs3_baseline() {
    let elf_path = std::path::Path::new("../../tests/micro/dma_completion/build/spu_main.elf");
    if !elf_path.exists() {
        return;
    }

    let baseline_dir = std::path::Path::new("../../baselines/dma_completion");
    let interp_path = baseline_dir.join("rpcs3_interpreter.json");
    let llvm_path = baseline_dir.join("rpcs3_llvm.json");
    if !interp_path.exists() || !llvm_path.exists() {
        return;
    }

    let elf_data = std::fs::read(elf_path).unwrap();
    let result_ea: u64 = 0x1_0000;

    let factory = || {
        let elf = elf_data.clone();
        cellgov_testkit::fixtures::ScenarioFixture::builder()
            .memory_size(0x2_0000)
            .budget(Budget::new(100_000))
            .max_steps(1_000)
            .register(move |rt| {
                let data = elf;
                rt.registry_mut().register_with(|id| {
                    let mut unit = SpuExecutionUnit::new(id);
                    loader::load_spu_elf(&data, unit.state_mut()).unwrap();
                    unit.state_mut().pc = 0x80;
                    unit.state_mut().set_reg_word_splat(1, 0x3FFF0);
                    unit.state_mut().set_reg_word_splat(4, result_ea as u32);
                    unit
                });
            })
            .build()
    };

    // header: 8 bytes at EA+0, pattern: 128 bytes at EA+16
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

    let cellgov_obs = cellgov_compare::observe_with_determinism_check(factory, &regions).unwrap();
    assert_eq!(
        cellgov_obs.outcome,
        cellgov_compare::ObservedOutcome::Completed
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
        "dma_completion diverges from RPCS3: {:?}",
        result.cellgov_result
    );
}

#[test]
fn spu_fixed_value_matches_rpcs3_baseline() {
    let elf_path = std::path::Path::new("../../tests/micro/spu_fixed_value/build/spu_main.elf");
    if !elf_path.exists() {
        return; // skip if not built
    }

    let baseline_dir = std::path::Path::new("../../baselines/spu_fixed_value");
    let interp_path = baseline_dir.join("rpcs3_interpreter.json");
    let llvm_path = baseline_dir.join("rpcs3_llvm.json");
    if !interp_path.exists() || !llvm_path.exists() {
        return; // skip if baselines not collected
    }

    let elf_data = std::fs::read(elf_path).unwrap();
    let result_ea: u64 = 0x1_0000;

    // Run through the full runtime via ScenarioFixture.
    let factory = || {
        let elf = elf_data.clone();
        cellgov_testkit::fixtures::ScenarioFixture::builder()
            .memory_size(0x2_0000)
            .budget(Budget::new(10_000))
            .max_steps(1_000)
            .register(move |rt| {
                let data = elf;
                rt.registry_mut().register_with(|id| {
                    let mut unit = SpuExecutionUnit::new(id);
                    loader::load_spu_elf(&data, unit.state_mut()).unwrap();
                    unit.state_mut().pc = 0x80;
                    unit.state_mut().set_reg_word_splat(1, 0x3FFF0);
                    unit.state_mut().set_reg_word_splat(4, result_ea as u32);
                    unit
                });
            })
            .build()
    };

    let regions = vec![cellgov_compare::RegionDescriptor {
        name: "result".into(),
        addr: result_ea,
        size: 8,
    }];

    let cellgov_obs = cellgov_compare::observe_with_determinism_check(factory, &regions).unwrap();
    assert_eq!(
        cellgov_obs.outcome,
        cellgov_compare::ObservedOutcome::Completed
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
        "spu_fixed_value diverges from RPCS3: {:?}",
        result.cellgov_result
    );
}
