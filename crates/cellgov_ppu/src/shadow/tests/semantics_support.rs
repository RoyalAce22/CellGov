//! Generated machine states and instruction runners for the shadow
//! semantics properties.

use proptest::prelude::*;

use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;
use cellgov_effects::Effect;
use cellgov_event::UnitId;

/// Guest address of the one readable region every case sees.
pub(super) const MEM_BASE: u64 = 0x1_0000;
/// Bytes in that region.
pub(super) const MEM_LEN: usize = 0x1_0000;
/// Program counter every case starts at; outside the region, so a
/// branch target never aliases a load address.
pub(super) const PC: u64 = 0x20_0000;

/// The inputs that fix one machine state and its memory.
#[derive(Debug, Clone)]
pub(super) struct StateSeed {
    gpr: Vec<u64>,
    cr: u32,
    lr: u64,
    ctr: u64,
    xer: u64,
    memory: u64,
}

/// Register values weighted toward addresses inside the region, so
/// loads and stores reach memory as often as they fault.
fn gpr_value() -> impl Strategy<Value = u64> {
    prop_oneof![
        4 => (0u64..(MEM_LEN as u64 - 64)).prop_map(|offset| MEM_BASE + (offset & !3)),
        1 => any::<u64>(),
        1 => Just(0u64),
        1 => Just(u64::MAX),
        2 => (-8i64..8).prop_map(|v| v as u64),
    ]
}

pub(super) fn state_seed() -> impl Strategy<Value = StateSeed> {
    (
        proptest::collection::vec(gpr_value(), 32),
        any::<u32>(),
        any::<u64>(),
        any::<u64>(),
        any::<u64>(),
        any::<u64>(),
    )
        .prop_map(|(gpr, cr, lr, ctr, xer, memory)| StateSeed {
            gpr,
            cr,
            lr,
            ctr,
            xer,
            memory,
        })
}

impl StateSeed {
    /// A fresh state with the seed's registers; two calls give two
    /// equal states.
    pub(super) fn state(&self) -> PpuState {
        let mut state = PpuState::new();
        let mut gpr = [0u64; 32];
        gpr.copy_from_slice(&self.gpr);
        state.set_gpr_all(gpr);
        state.set_cr(self.cr);
        state.set_lr(self.lr);
        state.set_ctr(self.ctr);
        state.set_xer(self.xer);
        state.pc = PC;
        state
    }

    /// The region's bytes: a SplitMix64 stream from the seed. Every
    /// other doubleword is a value in `-8..8`, so a loaded word often
    /// lands on a compare immediate.
    pub(super) fn memory(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(MEM_LEN);
        let mut x = self.memory;
        while out.len() < MEM_LEN {
            x = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
            let mut z = x;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
            let z = z ^ (z >> 31);
            let word = if z & 1 == 0 {
                z
            } else {
                (((z >> 8) & 0xF) as i64 - 8) as u64
            };
            out.extend_from_slice(&word.to_be_bytes());
        }
        out
    }
}

pub(super) fn uid() -> UnitId {
    UnitId::new(0)
}

/// Run `insns` in order the way the fetch loop does: a `Continue`
/// advances the PC by four and the next instruction runs. Any other
/// verdict ends the sequence. The store buffer flushes into the
/// returned effects.
pub(super) fn run_sequence(
    insns: &[PpuInstruction],
    state: &mut PpuState,
    mem: &[u8],
) -> (ExecuteVerdict, Vec<Effect>) {
    let views = [cellgov_mem::RegionView::plain(MEM_BASE, mem)];
    let mut store_buf = StoreBuffer::new();
    let mut effects = Vec::new();
    let mut verdict = ExecuteVerdict::Continue;
    for insn in insns {
        verdict = execute(insn, state, uid(), &views, &mut effects, &mut store_buf);
        if verdict != ExecuteVerdict::Continue {
            break;
        }
        state.pc += 4;
    }
    store_buf.flush(&mut effects, uid());
    (verdict, effects)
}

/// Run one fused pair: a `Continue` retires the pair and its
/// `Consumed` slot, eight bytes.
pub(super) fn run_pair(
    fused: &PpuInstruction,
    state: &mut PpuState,
    mem: &[u8],
) -> (ExecuteVerdict, Vec<Effect>) {
    let views = [cellgov_mem::RegionView::plain(MEM_BASE, mem)];
    let mut store_buf = StoreBuffer::new();
    let mut effects = Vec::new();
    let verdict = execute(fused, state, uid(), &views, &mut effects, &mut store_buf);
    if verdict == ExecuteVerdict::Continue {
        state.pc += 8;
    }
    store_buf.flush(&mut effects, uid());
    (verdict, effects)
}

pub(super) fn reg() -> impl Strategy<Value = u8> {
    0u8..32
}

pub(super) fn cr_field() -> impl Strategy<Value = u8> {
    0u8..8
}

/// A D-form displacement, biased toward small values so an address
/// register inside the region stays inside it.
pub(super) fn displacement() -> impl Strategy<Value = i16> {
    prop_oneof![
        3 => -256i16..256,
        1 => any::<i16>(),
    ]
}

/// A compare immediate, biased toward the small values the memory
/// stream and the register strategy produce.
pub(super) fn compare_immediate() -> impl Strategy<Value = i16> {
    prop_oneof![
        3 => -8i16..8,
        1 => any::<i16>(),
    ]
}

/// A DS-form displacement: the low two bits are zero.
pub(super) fn ds_displacement() -> impl Strategy<Value = i16> {
    displacement().prop_map(|d| d & !3)
}
