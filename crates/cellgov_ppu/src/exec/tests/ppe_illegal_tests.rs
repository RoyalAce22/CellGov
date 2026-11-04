//! The defined PowerPC instructions the PPE does not implement decode to
//! a named kind and execute to an illegal-instruction fault that changes
//! nothing.

use crate::exec::fault::PpuFault;
use crate::exec::test_support::uid;
use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::{ppe_illegal_mnemonic, PpuInstruction};
use crate::observation::PpuArchitecturalState;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// The opcode-31 X-form encodings: (extended opcode, mnemonic).
const X_FORMS: [(u32, &str); 7] = [
    (310, "eciwx"),
    (438, "ecowx"),
    (370, "tlbia"),
    (210, "mtsr"),
    (242, "mtsrin"),
    (595, "mfsr"),
    (659, "mfsrin"),
];

fn x_word(xo: u32, t: u32, a: u32, b: u32, rc: u32) -> u32 {
    (31 << 26) | (t << 21) | (a << 16) | (b << 11) | (xo << 1) | rc
}

fn a_word(primary: u32, xo_5: u32, frc: u32, rc: u32) -> u32 {
    (primary << 26) | (3 << 21) | (4 << 16) | (5 << 11) | (frc << 6) | (xo_5 << 1) | rc
}

/// Every PPE-unimplemented word this file checks, with its mnemonic.
fn illegal_words() -> Vec<(u32, &'static str)> {
    let mut words = Vec::new();
    for (xo, mnemonic) in X_FORMS {
        for (t, a, b) in [(0, 0, 0), (31, 31, 31), (3, 4, 5)] {
            for rc in 0..2 {
                words.push((x_word(xo, t, a, b, rc), mnemonic));
            }
        }
    }
    for frc in 0..32 {
        for rc in 0..2 {
            words.push((a_word(63, 24, frc, rc), "fre"));
            words.push((a_word(59, 26, frc, rc), "frsqrtes"));
        }
    }
    words.push((x_word(512, 5 << 2, 0, 0, 0), "mcrxr"));
    words.push((x_word(122, 4, 3, 0, 0), "popcntb"));
    words
}

/// A state with every register bank, CR and XER away from reset.
fn busy_state() -> PpuState {
    let mut state = PpuState::new();
    state.set_gpr_all(core::array::from_fn(|i| 0x0101_0101_0101_0101 * i as u64));
    for k in 0..32 {
        state.set_fpr(k, 0x3FF0_0000_0000_0000 | k as u64);
        state.set_vr(k, u128::MAX - k as u128);
    }
    state.set_cr(0x1234_5678);
    state.set_xer(0xE000_0000 | 0x7F);
    state.set_lr(0x1000);
    state.set_ctr(7);
    // [PPC-Book3 p:63 s:5.4] An interrupt does not clear a held reservation.
    state.set_reservation(Some(cellgov_sync::ReservedLine::containing(0x80)));
    state
}

#[test]
fn every_ppe_unimplemented_encoding_decodes_to_its_named_kind() {
    let words = illegal_words();
    assert_eq!(words.len(), 7 * 3 * 2 + 32 * 2 * 2 + 2);
    for (raw, mnemonic) in words {
        let insn = crate::decode::decode(raw).unwrap_or_else(|e| panic!("0x{raw:08x}: {e:?}"));
        match (mnemonic, insn) {
            ("mcrxr", PpuInstruction::Mcrxr { .. })
            | ("popcntb", PpuInstruction::Popcntb { .. }) => {}
            (_, PpuInstruction::PpeIllegal { raw: kept }) => {
                assert_eq!(kept, raw);
                assert_eq!(ppe_illegal_mnemonic(raw), Some(mnemonic), "0x{raw:08x}");
            }
            _ => panic!("0x{raw:08x} ({mnemonic}) decoded as {insn:?}"),
        }
        assert_eq!(
            crate::instruction::encode::encode(&insn),
            Ok(raw),
            "0x{raw:08x} encodes back to its word"
        );
    }
}

#[test]
fn the_implemented_neighbours_of_fre_and_frsqrtes_still_decode() {
    // [PPC-Book1 p:206 s:Appendix J] frsqrte is primary 63 A-form XO 26, and fres is primary 59 XO 24.
    for (primary, xo_5) in [(63, 26), (59, 24)] {
        let insn = crate::decode::decode(a_word(primary, xo_5, 0, 0)).expect("decodes");
        assert!(
            matches!(
                insn,
                PpuInstruction::Fp63 { .. } | PpuInstruction::Fp59 { .. }
            ),
            "{insn:?}"
        );
    }
}

#[test]
fn every_ppe_unimplemented_encoding_faults_and_changes_nothing() {
    let page = [0u8; 0x1000];
    let views = [cellgov_mem::RegionView::plain(0, &page)];
    for (raw, mnemonic) in illegal_words() {
        let insn = crate::decode::decode(raw).expect("decodes");
        let mut state = busy_state();
        let before = PpuArchitecturalState::capture(&state);
        let hash = state.state_hash();
        let mut effects = Vec::new();
        let mut stores = StoreBuffer::new();
        let verdict = execute(&insn, &mut state, uid(), &views, &mut effects, &mut stores);
        assert_eq!(
            verdict,
            ExecuteVerdict::Fault(PpuFault::IllegalInstruction(mnemonic)),
            "0x{raw:08x}"
        );
        assert_eq!(
            PpuArchitecturalState::capture(&state),
            before,
            "0x{raw:08x}"
        );
        assert_eq!(state.state_hash(), hash, "0x{raw:08x}");
        assert!(effects.is_empty(), "0x{raw:08x}");
        assert!(stores.is_empty(), "0x{raw:08x}");
    }
}

#[test]
fn mcrxr_leaves_cr_and_xer_as_they_were_for_every_status_nibble() {
    for nibble in 0..16u64 {
        for bf in 0..8u8 {
            let mut state = PpuState::new();
            state.set_xer((nibble << 28) | 0x7F);
            state.set_cr(0x8765_4321);
            let verdict =
                crate::exec::test_support::exec_no_mem(&PpuInstruction::Mcrxr { bf }, &mut state);
            assert_eq!(
                verdict,
                ExecuteVerdict::Fault(PpuFault::IllegalInstruction("mcrxr"))
            );
            assert_eq!(state.xer(), (nibble << 28) | 0x7F);
            assert_eq!(state.cr(), 0x8765_4321);
        }
    }
}

#[test]
fn a_ppe_unimplemented_word_disassembles_with_its_operands() {
    let cases = [
        (x_word(310, 3, 4, 5, 0), "eciwx r3, r4, r5"),
        (x_word(438, 3, 4, 5, 0), "ecowx r3, r4, r5"),
        (x_word(370, 0, 0, 0, 0), "tlbia"),
        (x_word(210, 3, 7, 0, 0), "mtsr 7, r3"),
        (x_word(595, 3, 7, 0, 0), "mfsr r3, 7"),
        (x_word(242, 3, 0, 5, 0), "mtsrin r3, r5"),
        (x_word(659, 3, 0, 5, 0), "mfsrin r3, r5"),
        (a_word(63, 24, 9, 1), "fre. f3, f5"),
        (a_word(59, 26, 0, 0), "frsqrtes f3, f5"),
    ];
    for (raw, want) in cases {
        let insn = crate::decode::decode(raw).expect("decodes");
        let text = crate::instruction::AsmText {
            insn: &insn,
            addr: 0,
            symbols: None,
        }
        .to_string();
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert_eq!(text, want, "0x{raw:08x}");
    }
}

#[test]
fn the_illegal_instruction_fault_has_its_own_guest_code() {
    let code = PpuFault::IllegalInstruction("mcrxr").guest_code();
    assert_eq!(code, crate::FAULT_ILLEGAL_INSTRUCTION);
    assert_ne!(
        code,
        PpuFault::UnimplementedInstruction {
            kind: crate::instruction::PpuInstructionKind::Vx,
            mnemonic: "vx",
        }
        .guest_code()
    );
    assert_ne!(code, PpuFault::InvalidForm("x").guest_code());
    assert!(!crate::is_decode_error(code));
}
