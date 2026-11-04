//! `vaddfp` held to the words the `ppu_vscr_mode` capture took under
//! each NJ mode.

use super::lane_support::pack_u32x4;
use super::vscr_console_tests::console_words;
use crate::exec::test_support::exec_no_mem;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use cellgov_ps3_abi::hw::ppu::VSCR_NJ;

/// The probe's three vaddfp operand pairs, at payload words 5, 6 and 8
/// under NJ = 0 and 10, 11 and 13 under NJ = 1.
const CASES: [(usize, u32, u32); 3] = [
    (0, 0x3F80_0000, 0x33C0_0000),
    (1, 0x7F80_0000, 0xFF80_0000),
    (3, 0x0000_0001, 0x0000_0000),
];

#[test]
fn vaddfp_gives_each_word_the_console_wrote_in_both_modes() {
    let console = console_words("ppu_vscr_mode", 19);
    for (base, nj) in [(5, false), (10, true)] {
        for (offset, a, b) in CASES {
            let mut s = PpuState::new();
            s.set_vscr(if nj { VSCR_NJ } else { 0 });
            s.set_vr(1, pack_u32x4([a; 4]));
            s.set_vr(2, pack_u32x4([b; 4]));
            let insn = PpuInstruction::Vx {
                op: VxOp::Vaddfp,
                rc: false,
                vt: 3,
                va: 1,
                vb: 2,
            };
            exec_no_mem(&insn, &mut s);
            assert_eq!(
                s.vr[3],
                pack_u32x4([console[base + offset]; 4]),
                "nj={nj} {a:#010x} + {b:#010x}"
            );
        }
    }
}
