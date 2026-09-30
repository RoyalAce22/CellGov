//! A unit executing predecoded code that a DMA transfer overwrites.
//!
//! The PPU's shadow caches decoded instructions. A transfer landing
//! over the text a unit runs from has to mark those slots stale, or the
//! unit keeps executing the instructions the transfer replaced. A title
//! that loads an overlay by MFC transfer reaches this.

#![allow(
    clippy::unwrap_used,
    reason = "integration test: a panic on unexpected failure is the right behavior"
)]

use cellgov_core::Runtime;
use cellgov_dma::{DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, UnitStatus, YieldReason,
};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::{Budget, InstructionCost};

/// Where the PPU loops.
const TEXT: u64 = 0x100;
/// Where the replacement instruction waits for the transfer.
const OVERLAY: u64 = 0x400;

/// `li r3, 1` and `li r3, 2`: `addi r3, r0, imm`.
const LI_R3_1: u32 = 0x3860_0001;
const LI_R3_2: u32 = 0x3860_0002;
/// `b -4`: back to the `li` from the word after it.
const B_BACK: u32 = 0x4BFF_FFFC;

fn range(addr: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), 4).unwrap()
}

fn place(mem: &mut GuestMemory, addr: u64, word: u32) {
    mem.apply_commit(range(addr), &word.to_be_bytes()).unwrap();
}

/// Enqueues one put of the word at [`OVERLAY`] over [`TEXT`], parks,
/// and finishes once the completion wakes it.
#[derive(Clone)]
struct Overlayer {
    id: UnitId,
    steps: u64,
}

impl ExecutionUnit for Overlayer {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        if self.steps >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.steps += 1;
        let yield_reason = if self.steps == 1 {
            let request =
                DmaRequest::new(DmaDirection::Put, range(OVERLAY), range(TEXT), self.id).unwrap();
            effects.push(Effect::DmaEnqueue {
                request,
                payload: None,
            });
            YieldReason::DmaWait
        } else {
            YieldReason::Finished
        };
        ExecutionStepResult {
            yield_reason,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn snapshot(&self) {}
}

/// A PPU looping on `li r3, 1` with a shadow over the loop, and the
/// overlayer beside it.
fn build() -> (Runtime, UnitId, UnitId) {
    let mut mem = GuestMemory::new(0x1000);
    place(&mut mem, TEXT, LI_R3_1);
    place(&mut mem, TEXT + 4, B_BACK);
    place(&mut mem, OVERLAY, LI_R3_2);
    let mut text = [0u8; 8];
    text[..4].copy_from_slice(&LI_R3_1.to_be_bytes());
    text[4..].copy_from_slice(&B_BACK.to_be_bytes());
    let shadow = PredecodedShadow::build(TEXT, &text);
    assert_eq!(
        shadow.get(TEXT).map(|_| ()),
        Some(()),
        "precondition: the shadow decoded the loop",
    );

    let mut rt = Runtime::new(mem, Budget::new(4), 64);
    let ppu = rt.register_unit_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        unit.state_mut().pc = TEXT;
        unit.set_instruction_shadow(shadow);
        unit
    });
    let overlayer = rt.register_unit_with(|id| Overlayer { id, steps: 0 });
    (rt, ppu, overlayer)
}

/// The overlayer finishes only after its completion wakes it, so its
/// status says when the transfer lands.
fn landed(rt: &Runtime, overlayer: UnitId) -> bool {
    rt.registry().effective_status(overlayer) == Some(UnitStatus::Finished)
}

/// Steps until the transfer lands, and checks the premise on the way:
/// while the loop's first word is still `li r3, 1`, r3 reads 1.
fn run_until_landed(rt: &mut Runtime, ppu: UnitId, overlayer: UnitId) {
    for _ in 0..32 {
        if landed(rt, overlayer) {
            break;
        }
        step_once(rt);
        let text = rt.memory().read(range(TEXT)).map(<[u8]>::to_vec);
        if text == Some(LI_R3_1.to_be_bytes().to_vec()) && rt.time().raw() > 4 {
            assert_eq!(
                r3(rt, ppu),
                1,
                "the premise: the loop loads 1 before the landing"
            );
        }
    }
    assert!(landed(rt, overlayer), "the transfer never landed");
    assert_eq!(
        rt.memory().read(range(TEXT)).map(<[u8]>::to_vec),
        Some(LI_R3_2.to_be_bytes().to_vec()),
        "the transfer replaced the loop's first word",
    );
}

fn r3(rt: &Runtime, ppu: UnitId) -> u64 {
    rt.registry()
        .get(ppu)
        .unwrap()
        .register_dump()
        .unwrap()
        .gprs[3]
}

fn step_once(rt: &mut Runtime) {
    let step = rt.step().expect("a unit is runnable");
    rt.commit_step(&step.result, &step.effects)
        .expect("no step of this workload refuses its commit");
}

#[test]
fn the_ppu_runs_the_instruction_the_transfer_landed() {
    let (mut rt, ppu, overlayer) = build();
    run_until_landed(&mut rt, ppu, overlayer);

    // The overlayer has finished, so the next steps are the PPU's.
    step_once(&mut rt);
    assert_eq!(
        r3(&rt, ppu),
        2,
        "the next block ran the instruction the transfer landed, not the \
         one the shadow had decoded",
    );
}

/// The value holds over the block after the landing too, so the loop
/// runs the landed instruction and the first block was no artefact of
/// the stale slot.
#[test]
fn the_loop_keeps_running_the_landed_instruction() {
    let (mut rt, ppu, overlayer) = build();
    run_until_landed(&mut rt, ppu, overlayer);
    step_once(&mut rt);
    step_once(&mut rt);
    assert_eq!(r3(&rt, ppu), 2, "and it stays 2 on the block after");
    let unit = rt
        .registry()
        .get(ppu)
        .unwrap()
        .as_any()
        .downcast_ref::<PpuExecutionUnit>()
        .unwrap();
    assert!(
        unit.state().pc == TEXT || unit.state().pc == TEXT + 4,
        "the loop is still the loop: pc {:#x}",
        unit.state().pc,
    );
}
