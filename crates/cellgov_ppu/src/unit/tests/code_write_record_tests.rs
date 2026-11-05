//! A store the PPU commits into its own declared text reaches the
//! runtime's record of guest stores into code.

use crate::instruction::encode::encode;
use crate::instruction::PpuInstruction as I;
use crate::*;
use cellgov_core::{AddressSpaceId, CodeWrite, Runtime};
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::lv2::syscall as sc;
use cellgov_time::Budget;

/// The word the program overwrites, inside its declared text.
const PATCHED: u64 = 0x40;

fn word(insn: I) -> u32 {
    encode(&insn).expect("encodable")
}

/// Runs a program that stores `ori 0,0,0` over [`PATCHED`] from
/// `rs = 5`, then exits, over text declared as `[0, text_len)`.
fn run(text_len: u64) -> (Runtime, UnitId) {
    let program = [
        word(I::Addis {
            rt: 5,
            ra: 0,
            imm: 0x6000,
        }),
        word(I::Stw {
            rs: 5,
            ra: 0,
            imm: PATCHED as i16,
        }),
        word(I::Addi {
            rt: 11,
            ra: 0,
            imm: sc::PROCESS_EXIT as i16,
        }),
        word(I::Sc { lev: 0 }),
    ];
    let mut mem = GuestMemory::new(0x100);
    for (i, w) in program.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
    let range = ByteRange::new(GuestAddr::new(PATCHED), 4).expect("range");
    mem.apply_commit(range, &0x4800_0000u32.to_be_bytes())
        .expect("mapped");
    mem.declare_code(0, text_len);
    let mut rt = Runtime::new(mem, Budget::new(1), 100);
    let unit = rt.register_unit_with(PpuExecutionUnit::new);
    for _ in 0..100 {
        let Ok(mut step) = rt.step() else { break };
        if rt.commit_step(&mut step).is_err() {
            break;
        }
    }
    (rt, unit)
}

/// The word at [`PATCHED`] after the run.
fn patched_word(rt: &Runtime) -> u32 {
    let range = ByteRange::new(GuestAddr::new(PATCHED), 4).expect("range");
    let b = rt.memory().read(range).expect("mapped");
    u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

#[test]
fn a_stw_into_declared_text_is_recorded_with_its_word_before_and_after() {
    let (rt, unit) = run(PATCHED + 4);
    let w = rt.code_writes();

    assert_eq!(w.stores(), 1);
    let [store] = w.detail() else {
        panic!("one store recorded, got {:?}", w.detail());
    };
    assert_eq!(
        *store,
        CodeWrite {
            unit,
            space: AddressSpaceId::BOOT,
            step: 2,
            word: PATCHED,
            before: 0x4800_0000,
            after: 0x6000_0000,
        },
        "the store is the second instruction"
    );
}

#[test]
fn a_stw_past_the_declared_text_is_not_recorded() {
    let (rt, _) = run(PATCHED);
    assert_eq!(patched_word(&rt), 0x6000_0000, "the stw committed");
    assert_eq!(rt.code_writes().stores(), 0);
}
