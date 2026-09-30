//! The state-hash accumulator stays equal to a full rehash through every
//! setter, a clone, a long instruction stream and a snapshot restore.

use super::*;
use crate::stop::SpuStopKind;
use cellgov_event::UnitId;
use cellgov_sync::ReservedLine;

fn assert_current(s: &SpuState, what: &str) {
    assert!(s.hash_is_current(), "{what}: accumulator out of date");
    assert_eq!(s.state_hash(), s.state_hash_from_scratch(), "{what}");
}

#[test]
fn a_new_state_starts_current() {
    assert_current(&SpuState::new(), "new");
    assert_current(&SpuState::default(), "default");
}

#[test]
fn every_hashed_setter_keeps_the_accumulator_current() {
    let mut s = SpuState::new();
    s.set_reg(0, [0xff; 16]);
    assert_current(&s, "set_reg r0");
    s.set_reg(127, (1u128 << 127).to_be_bytes());
    assert_current(&s, "set_reg r127");
    s.set_reg_all([[0x5a; 16]; SPU_REG_COUNT]);
    assert_current(&s, "set_reg_all");
    s.set_reg_word_splat(3, 0xdead_beef);
    assert_current(&s, "set_reg_word_splat");
    s.set_reg_channel_word(4, 0x1234);
    assert_current(&s, "set_reg_channel_word");
    s.set_reg_word_slot(5, 2, u32::MAX);
    assert_current(&s, "set_reg_word_slot");
    s.set_lslr(0x3_fff0);
    assert_current(&s, "set_lslr");
    s.set_fpscr(u128::MAX);
    assert_current(&s, "set_fpscr");
    s.set_fpscr(0);
    s.fpscr_accumulate_dbz([true, false, true, false]);
    assert_current(&s, "fpscr_accumulate_dbz");
    s.set_interrupts_enabled(true);
    assert_current(&s, "set_interrupts_enabled");
    s.set_srr0(0x100);
    assert_current(&s, "set_srr0");
    s.pc = 0x200;
    s.take_interrupt();
    assert_current(&s, "take_interrupt");
    s.set_reservation(Some(ReservedLine::containing(0x3000_1080)));
    assert_current(&s, "set_reservation Some");
    s.set_reservation(Some(ReservedLine::containing(0)));
    assert_current(&s, "set_reservation line 0");
    s.set_reservation(None);
    assert_current(&s, "set_reservation None");
}

#[test]
fn writing_a_lane_back_restores_its_hash() {
    let mut s = SpuState::new();
    s.set_reg(7, [42; 16]);
    let h = s.state_hash();
    s.set_reg(7, [42; 16]);
    assert_eq!(s.state_hash(), h, "a same-value write changes nothing");
    s.set_reg(7, [43; 16]);
    assert_ne!(s.state_hash(), h);
    s.set_reg(7, [42; 16]);
    assert_eq!(s.state_hash(), h);
}

#[test]
fn unhashed_writes_leave_the_accumulator_alone() {
    let mut s = SpuState::new();
    s.set_reg(3, [9; 16]);
    let h = s.state_hash();
    s.pc = 0x1000;
    s.ls[0x40] = 0xaa;
    s.signals[0].write(5);
    s.channels.in_mbox.push(6);
    s.channels.event_mask = 1;
    s.record_stop(SpuStopKind::Stop, 0x100);
    assert_eq!(s.state_hash(), h);
    assert_current(&s, "unhashed writes");
}

#[test]
fn a_clone_carries_the_accumulator() {
    let mut s = SpuState::new();
    s.set_reg(5, [0xab; 16]);
    s.set_fpscr(0x42);
    let c = s.clone();
    assert_current(&c, "clone");
    assert_eq!(c.state_hash(), s.state_hash());
}

/// SplitMix64 over `state`.
fn next(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

fn random_reg(rng: &mut u64) -> [u8; 16] {
    ((u128::from(next(rng)) << 64) | u128::from(next(rng))).to_be_bytes()
}

#[test]
fn a_long_random_sequence_of_writes_stays_current() {
    let mut rng = 0x5eed;
    let mut s = SpuState::new();
    for i in 0..20_000 {
        let v = next(&mut rng);
        match v % 9 {
            0..=3 => s.set_reg((v >> 8) as usize % SPU_REG_COUNT, random_reg(&mut rng)),
            4 => s.set_fpscr(u128::from_be_bytes(random_reg(&mut rng))),
            5 => s.set_srr0(next(&mut rng) as u32),
            6 => s.set_interrupts_enabled(v & 0x100 != 0),
            7 => s.set_reg_word_slot((v >> 8) as u8 & 0x7f, (v >> 16) as usize % 4, v as u32),
            _ => s.set_reservation(
                (v & 0x100 != 0)
                    .then(|| ReservedLine::containing(next(&mut rng) % 0x400_0000_0000)),
            ),
        }
        if i % 97 == 0 {
            assert_current(&s, "random sequence");
        }
    }
    assert_current(&s, "random sequence end");
}

#[test]
fn a_long_random_instruction_stream_keeps_the_accumulator_current() {
    let mut rng = 0x1551;
    let mut s = SpuState::new();
    for k in 0..SPU_REG_COUNT {
        s.set_reg(k, random_reg(&mut rng));
    }
    let mut executed = 0;
    for i in 0..200_000 {
        let raw = next(&mut rng) as u32;
        let Ok(insn) = crate::decode::decode(raw) else {
            continue;
        };
        // Each instruction runs from a fresh PC with any stop cleared.
        s.pc = (next(&mut rng) as u32) & 0x3_fffc;
        s.stop = None;
        let _ = crate::exec::execute(&insn, &mut s, UnitId::new(0));
        executed += 1;
        assert_current(&s, &format!("instruction {i}: {insn:?}"));
    }
    assert_current(&s, "instruction stream end");
    assert!(executed > 100_000, "{executed} instructions ran");
}
