//! Each SPU instruction the runtime retires leaves an `SpuStateHash`
//! record, and a full-state window leaves the fields that hash reads.

use cellgov_core::{Runtime, RuntimeMode};
use cellgov_event::UnitId;
use cellgov_exec::{SpuFingerprint, UnitStatus};
use cellgov_mem::GuestMemory;
use cellgov_spu::multilinear::{hash, lanes};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;
use cellgov_trace::{TraceReader, TraceRecord};

/// `il rt, imm`.
const fn il(rt: u32, imm: u32) -> u32 {
    (0x081 << 23) | (imm << 7) | rt
}

/// `wrch $ch<channel>, rt`.
const fn wrch(channel: u8, rt: u32) -> u32 {
    (0x10D << 21) | ((channel as u32) << 7) | rt
}

/// Three loads, then `stop 0`: four retired instructions.
const PROGRAM: [u32; 4] = [il(3, 1), il(4, 2), il(5, 3), 0];

/// A runtime in `mode` holding one SPU that runs [`PROGRAM`] with the
/// full-state window `window`, run until the SPU stops.
fn run(mode: RuntimeMode, window: Option<(u64, u64)>) -> (Runtime, UnitId) {
    run_program(&PROGRAM, mode, window)
}

fn run_program(
    program: &[u32],
    mode: RuntimeMode,
    window: Option<(u64, u64)>,
) -> (Runtime, UnitId) {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(2), 100);
    rt.set_mode(mode);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        for (i, word) in program.iter().enumerate() {
            spu.state_mut().ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        spu.set_full_state_window(window);
        spu
    });
    for _ in 0..20 {
        if rt.registry().effective_status(unit) == Some(UnitStatus::Finished) {
            break;
        }
        let step = rt.step().expect("the SPU runs");
        rt.commit_step(&step.result, &step.effects)
            .expect("the step commits");
    }
    assert_eq!(
        rt.registry().effective_status(unit),
        Some(UnitStatus::Finished)
    );
    (rt, unit)
}

fn records(bytes: &[u8]) -> Vec<TraceRecord> {
    TraceReader::new(bytes).map(Result::unwrap).collect()
}

/// `(unit, step, pc, hash)` of every `SpuStateHash` record.
fn hashes(bytes: &[u8]) -> Vec<(UnitId, u64, u64, u64)> {
    records(bytes)
        .into_iter()
        .filter_map(|r| match r {
            TraceRecord::SpuStateHash {
                unit,
                step,
                pc,
                hash,
            } => Some((unit, step, pc, hash.raw())),
            _ => None,
        })
        .collect()
}

fn spu(rt: &Runtime, unit: UnitId) -> &SpuExecutionUnit {
    rt.registry()
        .get(unit)
        .and_then(|u| u.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit")
}

#[test]
fn each_retired_spu_instruction_traces_its_state_hash() {
    let (rt, unit) = run(RuntimeMode::FullTrace, None);
    let h = hashes(rt.trace().bytes());
    let placed: Vec<(UnitId, u64, u64)> = h.iter().map(|&(u, s, pc, _)| (u, s, pc)).collect();
    assert_eq!(
        placed,
        [(unit, 0, 0), (unit, 1, 4), (unit, 2, 8), (unit, 3, 12)]
    );
    // Each load writes a hashed register.
    assert_ne!(h[0].3, h[1].3);
    assert_ne!(h[1].3, h[2].3);
    // The stop writes only the stopped state and the PC, which no lane
    // holds.
    assert_eq!(h[3].3, h[2].3);
    assert_eq!(h[3].3, spu(&rt, unit).state().state_hash());
}

#[test]
fn the_full_records_carry_every_input_of_the_hash() {
    let (rt, unit) = run(RuntimeMode::FullTrace, Some((1, 3)));
    let h = hashes(rt.trace().bytes());
    let zoom = records(rt.zoom_trace().bytes());
    let mut rebuilt = Vec::new();
    let mut at = 0;
    while at < zoom.len() {
        let TraceRecord::SpuStateFull {
            unit: u,
            step,
            pc,
            fpscr,
            lslr,
            interrupts_enabled,
            srr0,
            reservation_line,
        } = zoom[at]
        else {
            panic!("record {at} is not an SpuStateFull: {:?}", zoom[at]);
        };
        let mut regs = [0u128; 128];
        for block in 0..8 {
            let TraceRecord::SpuRegisters {
                unit: ru,
                step: rs,
                first,
                regs: chunk,
            } = zoom[at + 1 + block]
            else {
                panic!("record {} is not an SpuRegisters", at + 1 + block);
            };
            assert_eq!((ru, rs, first), (u, step, (block * 16) as u8));
            regs[block * 16..block * 16 + 16].copy_from_slice(&chunk);
        }
        let fingerprint = SpuFingerprint {
            regs,
            fpscr,
            lslr,
            interrupts_enabled,
            srr0,
            reservation_line,
        };
        rebuilt.push((u, step, pc, hash(&lanes(&fingerprint))));
        at += 9;
    }
    assert_eq!(rebuilt.len(), 3, "the window holds steps 1 through 3");
    assert_eq!(rebuilt, h[1..].to_vec());
    assert!(rebuilt.iter().all(|&(u, ..)| u == unit));
}

#[test]
fn a_fault_driven_run_traces_no_spu_state() {
    let (rt, _) = run(RuntimeMode::FaultDriven, Some((0, 3)));
    assert!(hashes(rt.trace().bytes()).is_empty());
    assert!(records(rt.zoom_trace().bytes())
        .iter()
        .all(|r| !matches!(r, TraceRecord::SpuStateFull { .. })));
}

#[test]
fn an_instruction_that_ends_the_step_is_traced_as_retired() {
    use cellgov_ps3_abi::hw::spu::{MFC_CMD, MFC_EAL, MFC_LSA, MFC_PUT, MFC_SIZE, MFC_TAG_ID};
    // The `MFC_Cmd` write queues a put and ends the step.
    let program = [
        il(3, 0x100),
        wrch(MFC_LSA, 3),
        il(3, 0x800),
        wrch(MFC_EAL, 3),
        il(3, 16),
        wrch(MFC_SIZE, 3),
        il(3, 0),
        wrch(MFC_TAG_ID, 3),
        il(3, MFC_PUT),
        wrch(MFC_CMD, 3),
        0,
    ];
    let (rt, unit) = run_program(&program, RuntimeMode::FullTrace, None);
    let placed: Vec<(UnitId, u64, u64)> = hashes(rt.trace().bytes())
        .iter()
        .map(|&(u, s, pc, _)| (u, s, pc))
        .collect();
    let want: Vec<(UnitId, u64, u64)> = (0..11).map(|k| (unit, k, 4 * k)).collect();
    assert_eq!(placed, want);
}
