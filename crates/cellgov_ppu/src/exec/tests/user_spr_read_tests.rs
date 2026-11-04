//! The CTRL and SPRG3 reads: decode, round trip, disassembly and the
//! value each writes to RT.

use super::*;

#[test]
fn ctrl_and_sprg3_reads_decode_round_trip_and_read_the_process_values() {
    use cellgov_ps3_abi::hw::ppu_spr::{PS3_PROCESS_CTRL, PS3_PROCESS_SPRG3};
    // mfspr r7, spr: the SPR's low half in RA, its high half in RB.
    let word =
        |spr: u32| (31 << 26) | (7 << 21) | ((spr & 0x1F) << 16) | ((spr >> 5) << 11) | (339 << 1);
    for (spr, insn, text, value) in [
        (
            136,
            PpuInstruction::Mfctrl { rt: 7 },
            "mfctrl r7",
            u64::from(PS3_PROCESS_CTRL),
        ),
        (
            259,
            PpuInstruction::Mfsprg3 { rt: 7 },
            "mfsprg3 r7",
            PS3_PROCESS_SPRG3,
        ),
    ] {
        let raw = word(spr);
        assert_eq!(crate::decode::decode(raw), Ok(insn), "{spr}");
        assert_eq!(crate::instruction::encode::encode(&insn), Ok(raw), "{spr}");
        let asm = crate::instruction::AsmText {
            insn: &insn,
            addr: 0,
            symbols: None,
        }
        .to_string();
        assert_eq!(asm.split_whitespace().collect::<Vec<_>>().join(" "), text);
        let mut s = PpuState::new();
        s.set_gpr(7, 0x5A5A_5A5A_5A5A_5A5A);
        let hash = s.state_hash();
        assert_eq!(exec_no_mem(&insn, &mut s), ExecuteVerdict::Continue);
        assert_eq!(s.gpr[7], value, "{spr}");
        let mut t = PpuState::new();
        t.set_gpr(7, value);
        assert_eq!(s.state_hash(), t.state_hash(), "{spr}: only RT changed");
        assert_ne!(hash, s.state_hash());
    }
}
