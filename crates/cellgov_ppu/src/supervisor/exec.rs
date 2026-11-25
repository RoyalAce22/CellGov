//! The privileged forms a thread in supervisor state executes.

use cellgov_ps3_abi::hw::ppu::CELL_BP_VR;
use cellgov_ps3_abi::hw::ppu_spr::{spr_read_class, spr_write_class, SprClass};
use cellgov_ps3_abi::hw::ppu_supervisor::{
    MSR_BIT1, MSR_DR, MSR_EE, MSR_HV, MSR_IR, MSR_ME, MSR_PR, MSR_RI, MSR_SE, MSR_SF,
    PPE_SLB_INDEX, PPE_SLB_VSID_FLAGS, SLB_ESID, SLB_VALID, SPR_BP_VR, SPR_DAR, SPR_DEC, SPR_DSISR,
    SPR_SPRG0, SPR_SRR0, SPR_SRR1,
};

use super::state::SupervisorState;
use crate::exec::{ExecuteVerdict, PpuFault};
use crate::instruction::{ppe_privileged_mnemonic, spr_number, PpuInstructionKind};
use crate::state::PpuState;

/// The mask of ISA bits `first..=last` of a 64-bit register.
const fn bits(first: u32, last: u32) -> u64 {
    (!0u64 >> first) & (!0u64 << (63 - last))
}

/// Whether `state` runs the privileged word `raw` as an `mfspr` or
/// `mtspr` of the decrementer, which reads the time base.
///
/// [CBE-Handbook p:254 s:9.5.9] In problem state an SPR move of a supervisor register takes a privileged-instruction interrupt, so it reads no time base.
pub(crate) fn moves_decrementer(raw: u32, state: &PpuState) -> bool {
    raw >> 26 == 31
        && matches!((raw >> 1) & 0x3FF, 339 | 467)
        && spr_number(raw) == SPR_DEC
        && state.supervisor().is_some_and(|s| !s.problem_state())
}

/// Run the privileged word `raw` on `state` in supervisor state.
///
/// When either condition holds, the function returns `None` and the
/// caller refuses the word as problem-state code:
///
/// - `state` carries no supervisor registers.
/// - MSR\[PR\] is 1.
pub(crate) fn execute(raw: u32, state: &mut PpuState) -> Option<ExecuteVerdict> {
    if state.supervisor()?.problem_state() {
        return None;
    }
    let mnemonic = ppe_privileged_mnemonic(raw).unwrap_or("ppe-privileged word");
    let rt = ((raw >> 21) & 0x1F) as usize;
    let rb = ((raw >> 11) & 0x1F) as usize;
    let l = (raw >> 16) & 1 == 1;
    let hypervisor = state.supervisor()?.msr & MSR_HV != 0;
    let verdict = match (raw >> 26, (raw >> 1) & 0x3FF) {
        (19, 18) => return Some(rfid(state)),
        (31, 83) => {
            let msr = state.supervisor()?.msr;
            state.set_gpr(rt, msr);
            ExecuteVerdict::Continue
        }
        (31, 146) => move_to_msr(state, rt, l, MtMsr::Word),
        (31, 178) => move_to_msr(state, rt, l, MtMsr::Doubleword),
        // [CBE-Handbook p:254 s:9.5.9] In supervisor state an access to a hypervisor SPR takes a privileged-instruction interrupt.
        (31, 339 | 467) if !hypervisor && moves_hypervisor_spr(raw) => {
            ExecuteVerdict::Fault(PpuFault::privileged(mnemonic))
        }
        (31, 339) => move_from_spr(state, rt, spr_number(raw), mnemonic),
        (31, 467) => move_to_spr(state, rt, spr_number(raw), mnemonic),
        (31, 402) => slb_move_to_entry(state, rt, rb),
        (31, 434) => {
            let esid = state.gpr[rb] & SLB_ESID;
            state.update_supervisor(|s| slb_invalidate(s, esid));
            ExecuteVerdict::Continue
        }
        (31, 498) => {
            state.update_supervisor(slb_invalidate_all);
            ExecuteVerdict::Continue
        }
        (31, 915) => slb_move_from_entry(state, rt, rb, Half::Effective),
        (31, 851) => slb_move_from_entry(state, rt, rb, Half::Virtual),
        // [PPC-Book3 p:13 s:2.3] hrfid in privileged, non-hypervisor state takes a privileged-instruction program interrupt or gives undefined results.
        // [PPC-Book3 p:54 s:4.11.3.2] tlbiel is privileged and hypervisor-only.
        // [PPC-Book3 p:53 s:4.11.3.2] tlbie runs only in hypervisor state; in privileged, non-hypervisor state it takes a privileged-instruction program interrupt or gives undefined results.
        // [PPC-Book3 p:56 s:4.11.3.2] tlbsync runs only in hypervisor state, with the same rule.
        (19, 274) | (31, 274 | 306 | 566) if !hypervisor => {
            ExecuteVerdict::Fault(PpuFault::privileged(mnemonic))
        }
        _ => unmodelled(mnemonic),
    };
    Some(verdict)
}

/// Whether the `mfspr` or `mtspr` word `raw` is an access of the
/// hypervisor class.
fn moves_hypervisor_spr(raw: u32) -> bool {
    let spr = spr_number(raw);
    let class = if (raw >> 1) & 0x3FF == 339 {
        spr_read_class(spr)
    } else {
        spr_write_class(spr)
    };
    class == SprClass::Hypervisor
}

/// The CellGov stop for a privileged form that supervisor state runs on
/// the console and the PPU does not model.
fn unmodelled(mnemonic: &'static str) -> ExecuteVerdict {
    ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction {
        kind: PpuInstructionKind::PpePrivileged,
        mnemonic,
    })
}

/// [PPC-Book3 p:13 s:2.3] rfid: SF from SRR1 bits 0 and 1; HV ANDed with SRR1 bit 3; ME from SRR1 only in hypervisor state; EE, IR and DR each ORed with SRR1 bit 49; bits 1:2, 4:32, 37:41, 49:50, 52:57 and 60:63 from SRR1; the next instruction at SRR0 bits 0:61.
fn rfid(state: &mut PpuState) -> ExecuteVerdict {
    let Some(s) = state.supervisor() else {
        return unmodelled("rfid");
    };
    let (old, srr1, srr0) = (s.msr, s.srr1, s.srr0);
    let pr = srr1 & MSR_PR != 0;
    let mut msr = srr1
        & (bits(1, 2) | bits(4, 32) | bits(37, 41) | bits(49, 50) | bits(52, 57) | bits(60, 63));
    msr |= (old & bits(33, 36)) | (old & bits(42, 47));
    if srr1 & (MSR_SF | MSR_BIT1) != 0 {
        msr |= MSR_SF;
    }
    msr |= old & srr1 & MSR_HV;
    msr |= if old & MSR_HV != 0 {
        srr1 & MSR_ME
    } else {
        old & MSR_ME
    };
    for bit in [MSR_EE, MSR_IR, MSR_DR] {
        if srr1 & bit != 0 || pr {
            msr |= bit;
        }
    }
    let nia = if msr & MSR_SF != 0 {
        srr0 & !3
    } else {
        srr0 & 0xFFFF_FFFC
    };
    state.update_supervisor(|s| s.msr = msr);
    state.pc = nia;
    ExecuteVerdict::Branch
}

/// Which of the two MSR moves runs.
#[derive(Clone, Copy)]
enum MtMsr {
    /// `mtmsr`, which writes the low 32 bits.
    Word,
    /// `mtmsrd`, which writes all 64.
    Doubleword,
}

/// [PPC-Book3 p:22 s:3.4] mtmsrd, L = 0: SF from RS bits 0 and 1; EE, IR and DR each ORed with RS bit 49; bits 1:2, 4:47, 49:50, 52:57 and 60:63 from RS; HV and ME unchanged. L = 1: only EE and RI.
/// [PPC-Book3 p:91 s:8.3] mtmsr, L = 0: EE, IR and DR as mtmsrd; bits 32:47, 49:50, 52:57 and 60:63 from RS; the high 32 bits and ME unchanged. L = 1 as mtmsrd.
///
/// [CBE-Handbook p:257 s:9.5.14] The PPE decides the trace after an mtmsrd from MSR\[SE\] after the move, the opposite of the architecture. The documents give no PPE rule for mtmsr, so CellGov refuses an mtmsr that changes SE.
fn move_to_msr(state: &mut PpuState, rs: usize, l: bool, form: MtMsr) -> ExecuteVerdict {
    let v = state.gpr[rs];
    let changes_se = state
        .supervisor()
        .is_some_and(|s| (s.msr ^ v) & MSR_SE != 0);
    if matches!(form, MtMsr::Word) && !l && changes_se {
        return ExecuteVerdict::Fault(PpuFault::Unmodelled("an mtmsr that changes MSR[SE]"));
    }
    state.update_supervisor(|s| {
        if l {
            s.msr = (s.msr & !(MSR_EE | MSR_RI)) | (v & (MSR_EE | MSR_RI));
            return;
        }
        let pr = v & MSR_PR != 0;
        let (copied, sf) = match form {
            MtMsr::Doubleword => (
                bits(1, 2) | bits(4, 47) | bits(49, 50) | bits(52, 57) | bits(60, 63),
                Some(v & (MSR_SF | MSR_BIT1) != 0),
            ),
            MtMsr::Word => (
                bits(32, 47) | bits(49, 50) | bits(52, 57) | bits(60, 63),
                None,
            ),
        };
        let kept = !(copied | MSR_EE | MSR_IR | MSR_DR | if sf.is_some() { MSR_SF } else { 0 });
        let mut msr = (s.msr & kept) | (v & copied);
        if sf == Some(true) {
            msr |= MSR_SF;
        }
        for bit in [MSR_EE, MSR_IR, MSR_DR] {
            if v & bit != 0 || pr {
                msr |= bit;
            }
        }
        s.msr = msr;
    });
    ExecuteVerdict::Continue
}

/// [PPC-Book3 p:21 s:3.4.2] mfspr: a 64-bit SPR moves whole; a 32-bit SPR moves into RT bits 32:63 with bits 0:31 cleared.
fn move_from_spr(
    state: &mut PpuState,
    rt: usize,
    spr: u16,
    mnemonic: &'static str,
) -> ExecuteVerdict {
    let Some(s) = state.supervisor() else {
        return unmodelled(mnemonic);
    };
    let v = match spr {
        SPR_DSISR => u64::from(s.dsisr),
        SPR_DAR => s.dar,
        SPR_DEC => {
            let dec = u64::from(s.dec_at(state.tb));
            state.clock_read = true;
            dec
        }
        SPR_SRR0 => s.srr0,
        SPR_SRR1 => s.srr1,
        _ if (SPR_SPRG0..SPR_SPRG0 + 4).contains(&spr) => s.sprg[usize::from(spr - SPR_SPRG0)],
        SPR_BP_VR => u64::from(CELL_BP_VR),
        _ => return unmodelled(mnemonic),
    };
    state.set_gpr(rt, v);
    ExecuteVerdict::Continue
}

/// [PPC-Book3 p:19 s:3.4.2] mtspr: a 64-bit SPR takes RS whole; a 32-bit SPR takes RS bits 32:63.
fn move_to_spr(
    state: &mut PpuState,
    rs: usize,
    spr: u16,
    mnemonic: &'static str,
) -> ExecuteVerdict {
    // [CBEA p:281 s:23.1] BP_VR provides no write access.
    // [CBEA p:312 s:Appendix E] E.4: in privileged state, an access to an inaccessible SPR executes as a no-op.
    if spr == SPR_BP_VR {
        return ExecuteVerdict::Continue;
    }
    let v = state.gpr[rs];
    let tb = state.tb;
    let modelled = matches!(spr, SPR_DSISR | SPR_DAR | SPR_DEC | SPR_SRR0 | SPR_SRR1)
        || (SPR_SPRG0..SPR_SPRG0 + 4).contains(&spr);
    if !modelled {
        return unmodelled(mnemonic);
    }
    if spr == SPR_DEC {
        state.clock_read = true;
    }
    state.update_supervisor(|s| match spr {
        SPR_DSISR => s.dsisr = v as u32,
        SPR_DAR => s.dar = v,
        SPR_DEC => {
            s.dec = v as u32;
            s.dec_tb = tb;
        }
        SPR_SRR0 => s.srr0 = v,
        SPR_SRR1 => s.srr1 = v,
        _ => s.sprg[usize::from(spr - SPR_SPRG0)] = v,
    });
    ExecuteVerdict::Continue
}

/// The SLB entry an SLB instruction's RB selects.
///
/// [CBE-Handbook p:84 s:4.2.5.2] The PPE ignores RB index bits 52:57, so every RB selects one of its 64 entries.
fn slb_index(rb_value: u64) -> usize {
    (rb_value & PPE_SLB_INDEX) as usize
}

/// [PPC-Book3 p:51 s:4.11.3.1] slbmte: the entry RB bits 52:63 select takes the ESID and V from RB and the VSID and flags from RS. RB index bits past the implemented SLB must be 0.
/// [CBE-Handbook p:84 s:4.2.5.2] The PPE also takes LP from RS bit 59, and ignores VSID bits 0:14.
fn slb_move_to_entry(state: &mut PpuState, rs: usize, rb: usize) -> ExecuteVerdict {
    let (rs_value, rb_value) = (state.gpr[rs], state.gpr[rb]);
    let index = slb_index(rb_value);
    state.update_supervisor(|s| {
        s.slb[index] = super::state::SlbEntry {
            esid_v: rb_value & (SLB_ESID | SLB_VALID),
            vsid_flags: rs_value & PPE_SLB_VSID_FLAGS,
        };
    });
    ExecuteVerdict::Continue
}

/// [PPC-Book3 p:49 s:4.11.3.1] slbie: the entry that translates ESID RB bits 0:35 becomes invalid, and its other fields undefined.
/// [CBE-Handbook p:84 s:4.2.5.2] On the PPE, slbie and slbia clear only V, and the other fields stay readable.
fn slb_invalidate(s: &mut SupervisorState, esid: u64) {
    for e in &mut s.slb {
        if e.valid() && e.esid_v & SLB_ESID == esid {
            e.esid_v &= !SLB_VALID;
        }
    }
}

/// [CBEA p:200 s:14.2.6] The PowerPC slbia instruction does not alter SLB entry zero; it invalidates every other entry.
fn slb_invalidate_all(s: &mut SupervisorState) {
    for e in &mut s.slb[1..] {
        e.esid_v &= !SLB_VALID;
    }
}

/// Which half of an SLB entry a move reads.
#[derive(Clone, Copy)]
enum Half {
    /// `slbmfee`: the ESID and V.
    Effective,
    /// `slbmfev`: the VSID and flags.
    Virtual,
}

/// [PPC-Book3 p:52 s:4.11.3.1] slbmfee places the selected entry's ESID and V in RT, and slbmfev its VSID and flags. For an invalid entry RT bit 36 is 0 and the rest undefined.
/// [CBE-Handbook p:84 s:4.2.5.2] The PPE keeps an invalidated entry's fields readable, so both moves read them whatever V holds.
fn slb_move_from_entry(state: &mut PpuState, rt: usize, rb: usize, half: Half) -> ExecuteVerdict {
    let index = slb_index(state.gpr[rb]);
    let Some(s) = state.supervisor() else {
        return unmodelled(match half {
            Half::Effective => "slbmfee",
            Half::Virtual => "slbmfev",
        });
    };
    let e = s.slb[index];
    let v = match half {
        Half::Effective => e.esid_v,
        Half::Virtual => e.vsid_flags,
    };
    state.set_gpr(rt, v);
    ExecuteVerdict::Continue
}
