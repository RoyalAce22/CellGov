//! Each privileged form runs on the supervisor registers with MSR\[PR\] = 0.
//! It refuses as problem-state code in either case:
//!
//! - MSR\[PR\] is 1.
//! - The thread carries no supervisor registers.

use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::ppu_supervisor::{
    MSR_DR, MSR_EE, MSR_HV, MSR_IR, MSR_ME, MSR_PR, MSR_RI, MSR_SF, SLB_VALID, SPR_DAR, SPR_DEC,
    SPR_DSISR, SPR_SPRG0, SPR_SRR0, SPR_SRR1,
};

use super::{moves_decrementer, SupervisorState};
use crate::exec::{execute, ExecuteVerdict, PpuFault};
use crate::instruction::PpuInstructionKind;
use crate::observation::PpuArchitecturalState;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// The kernel's MSR at its entry: 64-bit, privileged, translation off.
const KERNEL_MSR: u64 = MSR_SF;

fn word(primary: u32, xo: u32, t: u32, a: u32, b: u32) -> u32 {
    (primary << 26) | (t << 21) | (a << 16) | (b << 11) | (xo << 1)
}

fn mfspr(rt: u32, spr: u16) -> u32 {
    let spr = u32::from(spr);
    word(31, 339, rt, spr & 0x1F, spr >> 5)
}

fn mtspr(spr: u16, rs: u32) -> u32 {
    let spr = u32::from(spr);
    word(31, 467, rs, spr & 0x1F, spr >> 5)
}

fn kernel_state() -> PpuState {
    let mut state = PpuState::new();
    state.set_supervisor(Some(SupervisorState::new(KERNEL_MSR)));
    state
}

/// Execute `raw` on `state`, assert that it queued no effect or store
/// and kept the hash current, and return its verdict.
fn run(state: &mut PpuState, raw: u32) -> ExecuteVerdict {
    let insn = crate::decode::decode(raw).unwrap_or_else(|e| panic!("0x{raw:08x}: {e:?}"));
    let page = [0u8; 0x1000];
    let views = [cellgov_mem::RegionView::plain(0, &page)];
    let mut effects = Vec::new();
    let mut stores = StoreBuffer::new();
    let verdict = execute(
        &insn,
        state,
        UnitId::new(0),
        &views,
        &mut effects,
        &mut stores,
    );
    assert!(effects.is_empty() && stores.is_empty(), "0x{raw:08x}");
    assert!(state.hash_is_current(), "0x{raw:08x}");
    verdict
}

fn msr(state: &PpuState) -> u64 {
    state.supervisor().expect("supervisor registers").msr
}

/// The mask of ISA bits `first..=last`, bit 0 the most significant.
fn isa_bits(first: u32, last: u32) -> u64 {
    (first..=last).fold(0, |m, b| m | (1u64 << (63 - b)))
}

/// Run `rfid` from MSR `old` with SRR1 `srr1` and SRR0 `srr0`, and
/// return the new MSR and PC.
fn rfid_from(old: u64, srr1: u64, srr0: u64) -> (u64, u64) {
    let mut state = kernel_state();
    state.update_supervisor(|s| {
        s.msr = old;
        s.srr1 = srr1;
        s.srr0 = srr0;
    });
    assert_eq!(
        run(&mut state, word(19, 18, 0, 0, 0)),
        ExecuteVerdict::Branch
    );
    (msr(&state), state.pc)
}

#[test]
fn mfmsr_reads_the_msr_and_mtmsrd_with_l_0_writes_all_but_hv_and_me() {
    let mut state = kernel_state();
    state.update_supervisor(|s| s.msr = MSR_SF | MSR_HV | MSR_ME);
    // RS asks for HV = 0 and ME = 0, which mtmsrd leaves; RI and bit 1 set.
    state.set_gpr(3, MSR_RI | (1 << 62));
    assert_eq!(
        run(&mut state, word(31, 178, 3, 0, 0)),
        ExecuteVerdict::Continue
    );
    // Bit 1 ORs into SF; HV and ME stay; EE, IR and DR stay 0 with PR = 0.
    assert_eq!(msr(&state), MSR_SF | (1 << 62) | MSR_HV | MSR_ME | MSR_RI);
    assert_eq!(
        run(&mut state, word(31, 83, 4, 0, 0)),
        ExecuteVerdict::Continue
    );
    assert_eq!(state.gpr[4], msr(&state));
}

#[test]
fn mtmsrd_with_l_1_writes_only_ee_and_ri() {
    let mut state = kernel_state();
    state.set_gpr(3, !0);
    assert_eq!(
        run(&mut state, word(31, 178, 3, 1, 0)),
        ExecuteVerdict::Continue
    );
    assert_eq!(msr(&state), KERNEL_MSR | MSR_EE | MSR_RI);
    state.set_gpr(3, 0);
    run(&mut state, word(31, 178, 3, 1, 0));
    assert_eq!(msr(&state), KERNEL_MSR);
}

#[test]
fn an_msr_move_that_sets_pr_also_sets_ee_ir_and_dr() {
    let mut state = kernel_state();
    state.set_gpr(3, MSR_SF | MSR_PR);
    run(&mut state, word(31, 178, 3, 0, 0));
    assert_eq!(msr(&state), MSR_SF | MSR_PR | MSR_EE | MSR_IR | MSR_DR);
}

#[test]
fn mtmsr_writes_the_low_word_and_keeps_the_high_word_and_me() {
    let mut state = kernel_state();
    state.update_supervisor(|s| s.msr = MSR_SF | MSR_HV | MSR_ME);
    state.set_gpr(3, MSR_IR | MSR_DR);
    assert_eq!(
        run(&mut state, word(31, 146, 3, 0, 0)),
        ExecuteVerdict::Continue
    );
    assert_eq!(msr(&state), MSR_SF | MSR_HV | MSR_ME | MSR_IR | MSR_DR);
}

#[test]
fn rfid_restores_the_msr_from_srr1_and_branches_to_srr0() {
    let mut state = kernel_state();
    state.update_supervisor(|s| {
        s.msr = MSR_SF | MSR_ME;
        s.srr0 = 0x0001_0203;
        // A problem-state SRR1 with EE, IR and DR clear, and an interrupt
        // cause bit (44) the restore must not copy.
        s.srr1 = MSR_SF | MSR_PR | (1 << 19);
    });
    assert_eq!(
        run(&mut state, word(19, 18, 0, 0, 0)),
        ExecuteVerdict::Branch
    );
    assert_eq!(state.pc, 0x0001_0200);
    // PR forces EE, IR and DR; ME stays because HV = 0; bit 44 stays 0.
    assert_eq!(
        msr(&state),
        MSR_SF | MSR_PR | MSR_EE | MSR_IR | MSR_DR | MSR_ME
    );
}

#[test]
fn rfid_into_32_bit_mode_takes_the_low_word_of_srr0() {
    let mut state = kernel_state();
    state.update_supervisor(|s| {
        s.srr0 = 0x8000_0000_0000_1004;
        s.srr1 = 0;
    });
    run(&mut state, word(19, 18, 0, 0, 0));
    assert_eq!(state.pc, 0x1004);
    assert_eq!(msr(&state) & MSR_SF, 0);
}

#[test]
fn rfid_takes_only_the_srr1_bits_book3_lists_and_keeps_the_rest() {
    // Every SRR1 bit set from a privileged, non-hypervisor MSR: HV and ME
    // stay as they were (0), and bits 33:36 and 42:47 are not taken.
    let (msr, pc) = rfid_from(MSR_SF, !0, 0x0000_1234_5678_9ABF);
    assert_eq!(
        msr,
        !(MSR_HV | MSR_ME | isa_bits(33, 36) | isa_bits(42, 47))
    );
    assert_eq!(pc, 0x0000_1234_5678_9ABC);
    // Every SRR1 bit clear: the MSR keeps ME and bits 33:36 and 42:47,
    // and nothing else; SF = 0 takes the low word of SRR0.
    let kept = MSR_ME | isa_bits(33, 36) | isa_bits(42, 47);
    let (msr, pc) = rfid_from(kept | MSR_EE | MSR_IR | MSR_DR | MSR_RI, 0, !0);
    assert_eq!(msr, kept);
    assert_eq!(pc, 0xFFFF_FFFC);
}

#[test]
fn rfid_with_pr_0_takes_ee_ir_and_dr_each_from_srr1() {
    let (msr, _) = rfid_from(MSR_SF, MSR_SF | MSR_EE, 0);
    assert_eq!(msr, MSR_SF | MSR_EE);
    let (msr, _) = rfid_from(MSR_SF, MSR_SF | MSR_IR | MSR_DR, 0);
    assert_eq!(msr, MSR_SF | MSR_IR | MSR_DR);
}

#[test]
fn rfid_in_hypervisor_state_takes_me_from_srr1_and_ands_hv_with_it() {
    let (msr, _) = rfid_from(MSR_SF | MSR_HV | MSR_ME, MSR_SF | MSR_HV, 0);
    assert_eq!(msr, MSR_SF | MSR_HV);
    let (msr, _) = rfid_from(MSR_SF | MSR_HV, MSR_SF | MSR_ME, 0);
    assert_eq!(msr, MSR_SF | MSR_ME);
}

#[test]
fn mtmsrd_with_l_0_and_rs_0_clears_every_bit_but_hv_and_me() {
    let mut state = kernel_state();
    state.update_supervisor(|s| s.msr = !MSR_PR);
    state.set_gpr(3, 0);
    assert_eq!(
        run(&mut state, word(31, 178, 3, 0, 0)),
        ExecuteVerdict::Continue
    );
    assert_eq!(msr(&state), MSR_HV | MSR_ME);
}

#[test]
fn mtmsr_replaces_the_low_word_but_me_and_keeps_the_high_word() {
    let mut state = kernel_state();
    state.update_supervisor(|s| s.msr = MSR_SF | MSR_HV);
    state.set_gpr(3, !0);
    run(&mut state, word(31, 146, 3, 0, 0));
    assert_eq!(msr(&state), MSR_SF | MSR_HV | (0xFFFF_FFFF & !MSR_ME));
    let mut state = kernel_state();
    state.update_supervisor(|s| s.msr = !MSR_PR);
    state.set_gpr(3, 0);
    run(&mut state, word(31, 146, 3, 0, 0));
    assert_eq!(msr(&state), 0xFFFF_FFFF_0000_0000 | MSR_ME);
}

#[test]
fn each_supervisor_spr_moves_its_width_both_ways() {
    let wide = [
        SPR_DAR,
        SPR_SRR0,
        SPR_SRR1,
        SPR_SPRG0,
        SPR_SPRG0 + 1,
        SPR_SPRG0 + 2,
        SPR_SPRG0 + 3,
    ];
    for (k, spr) in wide.into_iter().enumerate() {
        let mut state = kernel_state();
        let v = 0x8000_0000_0000_0001 | ((k as u64) << 8);
        state.set_gpr(5, v);
        assert_eq!(run(&mut state, mtspr(spr, 5)), ExecuteVerdict::Continue);
        assert_eq!(run(&mut state, mfspr(6, spr)), ExecuteVerdict::Continue);
        assert_eq!(state.gpr[6], v, "spr {spr}");
    }
    let mut state = kernel_state();
    state.set_gpr(5, 0xFFFF_FFFF_8000_0001);
    run(&mut state, mtspr(SPR_DSISR, 5));
    run(&mut state, mfspr(6, SPR_DSISR));
    assert_eq!(state.gpr[6], 0x8000_0001);
}

#[test]
fn the_decrementer_counts_down_at_the_time_base() {
    let mut state = kernel_state();
    state.tb = 10;
    state.set_gpr(5, 1000);
    run(&mut state, mtspr(SPR_DEC, 5));
    assert!(state.clock_read);
    state.clock_read = false;
    state.tb = 110;
    run(&mut state, mfspr(6, SPR_DEC));
    assert_eq!(state.gpr[6], 900);
    assert!(state.clock_read);
    // Past zero it wraps through the 32-bit range.
    state.tb = 1011;
    run(&mut state, mfspr(6, SPR_DEC));
    assert_eq!(state.gpr[6], 0xFFFF_FFFF);
}

#[test]
fn slbmte_fills_an_entry_that_slbmfee_and_slbmfev_read_back() {
    let mut state = kernel_state();
    let esid_v = 0x8000_0000_0000_0000 | SLB_VALID;
    state.set_gpr(3, 0x0000_1234_5000_0400);
    state.set_gpr(4, esid_v | 5);
    assert_eq!(
        run(&mut state, word(31, 402, 3, 0, 4)),
        ExecuteVerdict::Continue
    );
    state.set_gpr(7, 5);
    run(&mut state, word(31, 915, 8, 0, 7));
    run(&mut state, word(31, 851, 9, 0, 7));
    assert_eq!(state.gpr[8], esid_v);
    assert_eq!(state.gpr[9], 0x0000_1234_5000_0400);
}

#[test]
fn slbie_invalidates_the_entry_of_its_esid_and_slbia_every_entry_but_zero() {
    let mut state = kernel_state();
    for index in [0u64, 1, 2] {
        state.set_gpr(3, index << 12);
        state.set_gpr(4, (index << 28) | SLB_VALID | index);
        run(&mut state, word(31, 402, 3, 0, 4));
    }
    state.set_gpr(4, 1 << 28);
    run(&mut state, word(31, 434, 0, 0, 4));
    let valid = |s: &PpuState| -> Vec<bool> {
        s.supervisor().unwrap().slb[..3]
            .iter()
            .map(|e| e.valid())
            .collect()
    };
    assert_eq!(valid(&state), [true, false, true]);
    run(&mut state, word(31, 498, 0, 0, 0));
    assert_eq!(valid(&state), [true, false, false]);
    // The PPE keeps an invalidated entry's ESID and VSID readable, with V = 0.
    state.set_gpr(7, 2);
    run(&mut state, word(31, 915, 8, 0, 7));
    run(&mut state, word(31, 851, 9, 0, 7));
    assert_eq!(state.gpr[8], 2 << 28);
    assert_eq!(state.gpr[9], 2 << 12);
    state.set_gpr(7, 1);
    run(&mut state, word(31, 915, 8, 0, 7));
    assert_eq!(state.gpr[8], 1 << 28);
}

#[test]
fn the_ppe_ignores_slb_index_bits_past_its_64_entries() {
    let mut state = kernel_state();
    state.set_gpr(3, 0x40 << 12);
    state.set_gpr(4, SLB_VALID | 0xFC5);
    assert_eq!(
        run(&mut state, word(31, 402, 3, 0, 4)),
        ExecuteVerdict::Continue
    );
    assert!(state.supervisor().unwrap().slb[5].valid());
    state.set_gpr(7, 0x45);
    assert_eq!(
        run(&mut state, word(31, 915, 8, 0, 7)),
        ExecuteVerdict::Continue
    );
    assert_eq!(state.gpr[8], SLB_VALID);
    run(&mut state, word(31, 851, 9, 0, 7));
    assert_eq!(state.gpr[9], 0x40 << 12);
}

#[test]
fn slbmte_keeps_the_ppe_lp_bit_and_drops_the_unimplemented_vsid_bits() {
    let mut state = kernel_state();
    state.set_gpr(3, !0);
    state.set_gpr(4, SLB_VALID);
    run(&mut state, word(31, 402, 3, 0, 4));
    state.set_gpr(7, 0);
    run(&mut state, word(31, 851, 9, 0, 7));
    assert_eq!(state.gpr[9], 0x0001_FFFF_FFFF_FF90);
}

#[test]
fn only_a_supervisor_state_decrementer_move_reads_the_time_base() {
    let mut state = kernel_state();
    assert!(moves_decrementer(mfspr(3, SPR_DEC), &state));
    assert!(moves_decrementer(mtspr(SPR_DEC, 3), &state));
    assert!(!moves_decrementer(mfspr(3, SPR_DAR), &state));
    state.update_supervisor(|s| s.msr |= MSR_PR);
    assert!(!moves_decrementer(mfspr(3, SPR_DEC), &state));
    assert!(!moves_decrementer(mfspr(3, SPR_DEC), &PpuState::new()));
}

#[test]
fn hypervisor_only_forms_refuse_and_unmodelled_forms_stop() {
    let mut state = kernel_state();
    assert_eq!(
        run(&mut state, word(19, 274, 0, 0, 0)),
        ExecuteVerdict::Fault(PpuFault::privileged("hrfid"))
    );
    assert_eq!(
        run(&mut state, word(31, 274, 0, 0, 5)),
        ExecuteVerdict::Fault(PpuFault::privileged("tlbiel"))
    );
    assert_eq!(
        run(&mut state, word(31, 306, 0, 0, 5)),
        ExecuteVerdict::Fault(PpuFault::privileged("tlbie"))
    );
    assert_eq!(
        run(&mut state, word(31, 566, 0, 0, 0)),
        ExecuteVerdict::Fault(PpuFault::privileged("tlbsync"))
    );
    let unmodelled = |mnemonic| {
        ExecuteVerdict::Fault(PpuFault::UnimplementedInstruction {
            kind: PpuInstructionKind::PpePrivileged,
            mnemonic,
        })
    };
    // PVR is privileged and not one of the supervisor registers modelled.
    assert_eq!(run(&mut state, mfspr(3, 287)), unmodelled("mfspr"));
    assert_eq!(run(&mut state, mtspr(287, 3)), unmodelled("mtspr"));
    // In hypervisor state the PPU refuses no hypervisor-only form; it does
    // not model them.
    state.update_supervisor(|s| s.msr |= MSR_HV);
    assert_eq!(run(&mut state, word(19, 274, 0, 0, 0)), unmodelled("hrfid"));
    assert_eq!(run(&mut state, word(31, 306, 0, 0, 5)), unmodelled("tlbie"));
}

#[test]
fn in_problem_state_every_form_refuses_and_changes_nothing() {
    let mut state = kernel_state();
    state.update_supervisor(|s| s.msr = MSR_SF | MSR_PR | MSR_EE | MSR_IR | MSR_DR);
    state.set_gpr(3, 0x1234);
    let before = PpuArchitecturalState::capture(&state);
    let hash = state.state_hash();
    for (raw, mnemonic) in [
        (word(31, 83, 3, 0, 0), "mfmsr"),
        (word(31, 178, 3, 0, 0), "mtmsrd"),
        (word(19, 18, 0, 0, 0), "rfid"),
        (mtspr(SPR_SRR0, 3), "mtspr"),
        (word(31, 402, 3, 0, 4), "slbmte"),
    ] {
        assert_eq!(
            run(&mut state, raw),
            ExecuteVerdict::Fault(PpuFault::privileged(mnemonic))
        );
        assert_eq!(PpuArchitecturalState::capture(&state), before, "{mnemonic}");
        assert_eq!(state.state_hash(), hash, "{mnemonic}");
    }
}

#[test]
fn the_sprg3_read_alias_reads_the_kernels_sprg3_in_problem_state() {
    let mut state = kernel_state();
    state.update_supervisor(|s| {
        s.msr |= MSR_PR;
        s.sprg[3] = 0xABCD;
    });
    run(&mut state, mfspr(4, 259));
    assert_eq!(state.gpr[4], 0xABCD);
    let mut title = PpuState::new();
    run(&mut title, mfspr(4, 259));
    assert_eq!(title.gpr[4], 0);
}

#[test]
fn supervisor_registers_move_the_hash_and_their_removal_restores_it() {
    let mut state = PpuState::new();
    let title = state.state_hash();
    state.set_supervisor(Some(SupervisorState::new(KERNEL_MSR)));
    let kernel = state.state_hash();
    assert_ne!(kernel, title);
    state.update_supervisor(|s| s.sprg[0] = 1);
    assert_ne!(state.state_hash(), kernel);
    assert!(state.hash_is_current());
    state.set_supervisor(None);
    assert_eq!(state.state_hash(), title);
    assert!(state.hash_is_current());
}

#[test]
fn a_snapshot_carries_the_supervisor_registers_through_capture_and_apply() {
    let mut state = kernel_state();
    state.update_supervisor(|s| s.srr0 = 0x40);
    let snapshot = PpuArchitecturalState::capture(&state);
    let mut restored = PpuState::new();
    snapshot.apply(&mut restored);
    assert_eq!(restored.supervisor(), state.supervisor());
    assert_eq!(restored.state_hash(), state.state_hash());
}
