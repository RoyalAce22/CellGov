//! The Book III instructions and the privileged and reserved SPR moves
//! decode to a named kind and execute to a privileged- or
//! illegal-instruction fault that changes nothing.

use cellgov_ps3_abi::hw::ppu_spr::{spr_read_class, spr_write_class, SprClass};

use crate::exec::fault::PpuFault;
use crate::exec::test_support::uid;
use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::known_encodings::{spr_gap, SprDirection};
use crate::instruction::{ppe_privileged_mnemonic, PpuDecodeError, PpuInstruction};
use crate::observation::PpuArchitecturalState;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

/// The Book III encodings: (primary, extended opcode, mnemonic).
const BOOK3: [(u32, u32, &str); 13] = [
    (19, 18, "rfid"),
    (19, 274, "hrfid"),
    (31, 83, "mfmsr"),
    (31, 146, "mtmsr"),
    (31, 178, "mtmsrd"),
    (31, 498, "slbia"),
    (31, 434, "slbie"),
    (31, 402, "slbmte"),
    (31, 851, "slbmfev"),
    (31, 915, "slbmfee"),
    (31, 306, "tlbie"),
    (31, 274, "tlbiel"),
    (31, 566, "tlbsync"),
];

fn word(primary: u32, xo: u32, t: u32, a: u32, b: u32) -> u32 {
    (primary << 26) | (t << 21) | (a << 16) | (b << 11) | (xo << 1)
}

/// `mfspr r3, spr` (XO 339) or `mtspr spr, r3` (XO 467).
fn spr_word(xo: u32, spr: u16) -> u32 {
    let spr = u32::from(spr);
    word(31, xo, 3, spr & 0x1F, spr >> 5)
}

/// Runs `raw` from a state away from reset and asserts the fault and
/// that nothing changed.
fn assert_refused(raw: u32, want: PpuFault) {
    let insn = crate::decode::decode(raw).unwrap_or_else(|e| panic!("0x{raw:08x}: {e:?}"));
    let page = [0u8; 0x1000];
    let views = [cellgov_mem::RegionView::plain(0, &page)];
    let mut state = PpuState::new();
    state.set_gpr_all(core::array::from_fn(|i| 0x0101_0101_0101_0101 * i as u64));
    state.set_cr(0x1234_5678);
    state.set_lr(0x1000);
    let before = PpuArchitecturalState::capture(&state);
    let hash = state.state_hash();
    let mut effects = Vec::new();
    let mut stores = StoreBuffer::new();
    let verdict = execute(&insn, &mut state, uid(), &views, &mut effects, &mut stores);
    assert_eq!(verdict, ExecuteVerdict::Fault(want), "0x{raw:08x}");
    assert_eq!(
        PpuArchitecturalState::capture(&state),
        before,
        "0x{raw:08x}"
    );
    assert_eq!(state.state_hash(), hash, "0x{raw:08x}");
    assert!(effects.is_empty() && stores.is_empty(), "0x{raw:08x}");
}

#[test]
fn every_book3_encoding_decodes_to_its_named_kind_and_refuses() {
    for (primary, xo, mnemonic) in BOOK3 {
        for (t, a, b) in [(0, 0, 0), (31, 31, 31), (3, 4, 5)] {
            let raw = word(primary, xo, t, a, b);
            let insn = crate::decode::decode(raw).unwrap_or_else(|e| panic!("{mnemonic}: {e:?}"));
            assert_eq!(insn, PpuInstruction::PpePrivileged { raw }, "{mnemonic}");
            assert_eq!(ppe_privileged_mnemonic(raw), Some(mnemonic));
            assert_eq!(crate::instruction::encode::encode(&insn), Ok(raw));
            assert_refused(raw, PpuFault::PrivilegedInstruction(mnemonic));
        }
    }
}

/// [PPC-Book3 p:21 s:3.4.2] mfspr and [PPC-Book3 p:19 s:3.4.2] mtspr are user, privileged or reserved by SPR number.
#[test]
fn every_spr_number_decodes_by_its_class_in_both_directions() {
    let mut user = [0; 2];
    let mut gaps = 0;
    for spr in 0..1024u16 {
        for (dir, (xo, class, mnemonic, direction)) in [
            (339, spr_read_class(spr), "mfspr", SprDirection::MfSpr),
            (467, spr_write_class(spr), "mtspr", SprDirection::MtSpr),
        ]
        .into_iter()
        .enumerate()
        {
            let raw = spr_word(xo, spr);
            match (class, crate::decode::decode(raw)) {
                (SprClass::Privileged, Ok(PpuInstruction::PpePrivileged { .. })) => {
                    assert_refused(raw, PpuFault::PrivilegedInstruction(mnemonic));
                }
                (SprClass::Reserved, Ok(PpuInstruction::PpeIllegal { .. })) => {
                    assert_refused(raw, PpuFault::IllegalInstruction(mnemonic));
                }
                (SprClass::User, Ok(insn)) => {
                    assert!(
                        !matches!(
                            insn,
                            PpuInstruction::PpePrivileged { .. }
                                | PpuInstruction::PpeIllegal { .. }
                        ),
                        "{mnemonic} {spr}: {insn:?}"
                    );
                    user[dir] += 1;
                }
                (SprClass::User, Err(PpuDecodeError::DecoderArmUnimplemented { .. })) => {
                    assert!(spr_gap(direction, spr).is_some(), "{mnemonic} {spr}");
                    gaps += 1;
                }
                (class, got) => panic!("{mnemonic} {spr} ({class:?}): {got:?}"),
            }
        }
    }
    // XER, LR, CTR, VRSAVE, TB and TBU read; XER, LR, CTR and VRSAVE write.
    assert_eq!(user, [6, 4]);
    // The CTRL read and the SPRG3 read alias still want an arm.
    assert_eq!(gaps, 2);
}

#[test]
fn directed_spr_numbers_take_the_documented_refusal() {
    assert_refused(spr_word(339, 272), PpuFault::PrivilegedInstruction("mfspr"));
    assert_refused(spr_word(467, 310), PpuFault::PrivilegedInstruction("mtspr"));
    assert_refused(spr_word(339, 287), PpuFault::PrivilegedInstruction("mfspr"));
    // Each SPR here is reserved:
    // - 280: the v2.02 tables have no ASR.
    // - 282: the PPE has no EAR.
    // - 1008: no document on disk gives the number of HID0.
    for spr in [280, 282, 1008] {
        assert_refused(spr_word(339, spr), PpuFault::IllegalInstruction("mfspr"));
        assert_refused(spr_word(467, spr), PpuFault::IllegalInstruction("mtspr"));
    }
}

#[test]
fn a_privileged_word_disassembles_with_its_operands() {
    let cases = [
        (word(19, 18, 0, 0, 0), "rfid"),
        (word(19, 274, 0, 0, 0), "hrfid"),
        (word(31, 83, 3, 0, 0), "mfmsr r3"),
        (word(31, 146, 3, 1, 0), "mtmsr r3, 1"),
        (word(31, 178, 3, 1, 0), "mtmsrd r3, 1"),
        (word(31, 498, 0, 0, 0), "slbia"),
        (word(31, 434, 0, 0, 5), "slbie r5"),
        (word(31, 402, 3, 0, 5), "slbmte r3, r5"),
        (word(31, 851, 3, 0, 5), "slbmfev r3, r5"),
        (word(31, 915, 3, 0, 5), "slbmfee r3, r5"),
        (word(31, 306, 1, 0, 5), "tlbie r5, 1"),
        (word(31, 274, 0, 0, 5), "tlbiel r5, 0"),
        (word(31, 566, 0, 0, 0), "tlbsync"),
        (spr_word(339, 272), "mfspr r3, 272"),
        (spr_word(467, 310), "mtspr 310, r3"),
        (spr_word(339, 280), "mfspr r3, 280"),
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
fn the_privileged_instruction_fault_has_its_own_guest_code() {
    let code = PpuFault::PrivilegedInstruction("mfmsr").guest_code();
    assert_eq!(code, crate::FAULT_PRIVILEGED_INSTRUCTION);
    assert_ne!(code, PpuFault::IllegalInstruction("x").guest_code());
    assert_ne!(code, PpuFault::InvalidForm("x").guest_code());
    assert!(!crate::is_decode_error(code));
}
