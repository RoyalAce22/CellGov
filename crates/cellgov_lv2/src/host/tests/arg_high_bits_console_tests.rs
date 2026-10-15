//! `sys_ppu_thread_set_priority` and `sys_ppu_thread_get_priority` held
//! to the `lv2_arg_high_bits` capture from the reference console: an
//! `int` argument whose register carries a high word sets its low word,
//! and an out pointer with bit 32 set answers CELL_EFAULT with nothing
//! written.

use cellgov_event::UnitId;
use cellgov_ps3_abi::lv2::syscall;

use super::*;
use crate::host::test_support::{
    console_payload, extract_write_u32, seed_primary_ppu, FakeRuntime,
};
use crate::ppu_thread::PpuThreadId;
use crate::request::Lv2Request;
use crate::Lv2Dispatch;

/// The guest slot `sys_ppu_thread_get_priority` writes.
const PRIO_PTR: u64 = 0x2000;

fn console() -> Vec<u64> {
    console_payload("lv2_arg_high_bits")
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect()
}

fn call(host: &mut Lv2Host, rt: &FakeRuntime, number: u64, arg1: u64) -> Lv2Dispatch {
    let mut args = [0u64; 8];
    args[0] = PpuThreadId::PRIMARY.raw();
    args[1] = arg1;
    host.dispatch(Lv2Request::Unsupported { number, args }, UnitId::new(0), rt)
}

fn read_priority(host: &mut Lv2Host, rt: &FakeRuntime) -> u64 {
    match call(host, rt, syscall::PPU_THREAD_GET_PRIORITY, PRIO_PTR) {
        Lv2Dispatch::Immediate { code: 0, effects } => u64::from(extract_write_u32(&effects[0])),
        other => panic!("get_priority: {other:?}"),
    }
}

fn host() -> (Lv2Host, FakeRuntime) {
    let rt = FakeRuntime::with_memory(cellgov_mem::GuestMemory::new(0x10000));
    let mut host = Lv2Host::new();
    seed_primary_ppu(&mut host, UnitId::new(0));
    (host, rt)
}

#[test]
fn a_priority_register_with_a_high_word_sets_its_low_word_as_on_the_console() {
    let words = console();
    let (mut host, rt) = host();
    for (register, rc_slot) in [(0x1_0000_0BB8u64, 6), (0xFFFF_FFFF_0000_0BB8, 9)] {
        let d = call(&mut host, &rt, syscall::PPU_THREAD_SET_PRIORITY, register);
        assert_eq!(
            d,
            Lv2Dispatch::immediate(words[rc_slot]),
            "set {register:#x}"
        );
        assert_eq!(
            read_priority(&mut host, &rt),
            words[rc_slot + 1],
            "read back after {register:#x}"
        );
        // The probe restores the original after each set, so the next
        // read-back separates a store of 3000 from a no-op CELL_OK.
        let d = call(&mut host, &rt, syscall::PPU_THREAD_SET_PRIORITY, words[3]);
        assert_eq!(d, Lv2Dispatch::immediate(words[rc_slot + 2]), "restore");
    }
}

#[test]
fn an_out_pointer_with_bit_32_set_faults_and_writes_nothing_as_on_the_console() {
    let words = console();
    let (mut host, rt) = host();
    let d = call(
        &mut host,
        &rt,
        syscall::PPU_THREAD_GET_PRIORITY,
        PRIO_PTR | (1 << 32),
    );
    assert_eq!(words[5], 0xEEEE_EEEE, "the console wrote nothing");
    assert_eq!(d, Lv2Dispatch::immediate(words[4]));
}
