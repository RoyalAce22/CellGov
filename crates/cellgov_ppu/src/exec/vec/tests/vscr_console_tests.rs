//! The VSCR held to the `ppu_vscr_sat` and `ppu_vscr_mode` captures.

use crate::exec::test_support::exec_no_mem;
use crate::instruction::ops::VxOp;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::unit::PpuExecutionUnit;
use cellgov_event::UnitId;

fn console_words(probe: &str, count: usize) -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u32> = frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect();
    assert_eq!(words.len(), count);
    assert_eq!(words[0], 0, "the probe ran to its end");
    words
}

fn splat(word: u32) -> u128 {
    let w = u128::from(word);
    w | (w << 32) | (w << 64) | (w << 96)
}

fn mtvscr(s: &mut PpuState, value: u128) {
    s.set_vr(30, value);
    exec_no_mem(&PpuInstruction::Mtvscr { vb: 30 }, s);
}

fn vx(s: &mut PpuState, op: VxOp, a: u128, b: u128) -> u32 {
    s.set_vr(1, a);
    s.set_vr(2, b);
    let insn = PpuInstruction::Vx {
        op,
        rc: false,
        vt: 3,
        va: 1,
        vb: 2,
    };
    exec_no_mem(&insn, s);
    (s.vr[3] >> 96) as u32
}

/// The probe's child starts while its parent holds NJ = 0; a new
/// thread's unit starts from a fresh state either way.
#[test]
fn a_new_thread_starts_with_the_vscr_the_console_thread_saw() {
    let console = console_words("ppu_vscr_mode", 19);
    assert_eq!(PpuState::new().vscr(), console[2], "main");
    let unit = PpuExecutionUnit::new(UnitId::new(1));
    assert_eq!(unit.state().vscr(), console[15], "child entry");
}

#[test]
fn mfvscr_after_mtvscr_gives_the_vector_the_console_read() {
    let console = console_words("ppu_vscr_sat", 15);
    let mut s = PpuState::new();
    mtvscr(&mut s, (u128::MAX << 32) | 0x0001_0001);
    s.set_vr(4, u128::MAX);
    exec_no_mem(&PpuInstruction::Mfvscr { vt: 4 }, &mut s);
    let expected = (u128::from(console[2]) << 96)
        | (u128::from(console[3]) << 64)
        | (u128::from(console[4]) << 32)
        | u128::from(console[5]);
    assert_eq!(s.vr[4], expected);
}

#[test]
fn the_reserved_bits_read_back_as_the_console_read_them() {
    let console = console_words("ppu_vscr_sat", 15);
    let mut s = PpuState::new();
    mtvscr(&mut s, splat(0xFFFF_FFFF));
    assert_eq!(s.vscr(), console[6], "all ones");
    mtvscr(&mut s, splat(0xA5A5_A5A5));
    assert_eq!(s.vscr(), console[7], "pattern");
}

#[test]
fn sat_follows_the_console_through_the_vsububs_and_vaddubm_steps() {
    let console = console_words("ppu_vscr_sat", 15);
    let fives = splat(0x0505_0505);
    let mut s = PpuState::new();

    mtvscr(&mut s, 0);
    let result = vx(&mut s, VxOp::Vsububs, fives, fives);
    assert_eq!(s.vscr(), console[8], "at the bound");
    assert_eq!(result, console[9], "at the bound, word 0");

    let one_clamps = (splat(0x0909_0909) & !(0xFF << 120)) | (0x03 << 120);
    vx(&mut s, VxOp::Vsububs, one_clamps, fives);
    assert_eq!(s.vscr(), console[10], "one lane clamps");

    vx(&mut s, VxOp::Vsububs, fives, fives);
    assert_eq!(s.vscr(), console[11], "sticky");

    mtvscr(&mut s, 0);
    assert_eq!(s.vscr(), console[12], "cleared");

    let result = vx(
        &mut s,
        VxOp::Vaddubm,
        splat(0xFFFF_FFFF),
        splat(0x0101_0101),
    );
    assert_eq!(s.vscr(), console[13], "modulo wrap");
    assert_eq!(result, console[14], "modulo wrap, word 0");
}
