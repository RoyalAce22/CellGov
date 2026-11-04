//! VRSAVE held to the `ppu_vrsave_entry` capture: the value a PPU thread
//! sees at entry, and the `mtvrsave` / `mfvrsave` round trip.

use crate::exec::test_support::exec_no_mem;
use crate::instruction::PpuInstruction;
use crate::state::PpuState;
use crate::unit::PpuExecutionUnit;
use cellgov_event::UnitId;

fn console_words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_vrsave_entry/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let words: Vec<u32> = frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect();
    assert_eq!(words.len(), 7);
    assert_eq!(words[0], 0, "the probe ran to its end");
    words
}

/// The probe's child starts while its parent holds VRSAVE 0xC0000001; a
/// new thread's unit starts from a fresh state either way.
#[test]
fn a_new_thread_starts_with_the_vrsave_the_console_thread_saw() {
    let console = console_words();
    let unit = PpuExecutionUnit::new(UnitId::new(1));
    assert_eq!(unit.state().vrsave(), console[5], "child entry");
    assert_eq!(PpuState::new().vrsave(), console[2], "main");
}

#[test]
fn the_vrsave_round_trip_gives_the_rt_the_console_read() {
    let console = console_words();
    let mut s = PpuState::new();
    s.set_gpr(9, 0xFFFF_FFFF_C000_0001);
    exec_no_mem(&PpuInstruction::Mtvrsave { rs: 9 }, &mut s);
    exec_no_mem(&PpuInstruction::Mfvrsave { rt: 9 }, &mut s);
    assert_eq!(s.gpr[9] as u32, console[3], "low word");
    assert_eq!((s.gpr[9] >> 32) as u32, console[4], "high word");
}
