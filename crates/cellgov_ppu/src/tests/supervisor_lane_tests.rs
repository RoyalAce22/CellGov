//! The supervisor lane: its keys, its pinned values, and that every bit
//! of every supervisor register reaches the state hash.

use super::*;
use crate::mmu::Partition;
use crate::state::PpuState;
use crate::supervisor::SupervisorState;
use cellgov_ps3_abi::hw::ppu_supervisor::PPE_SLB_ENTRIES;
use std::collections::BTreeSet;

#[test]
fn supervisor_keys_are_the_indexed_keys_of_their_seed() {
    for (k, key) in SUPERVISOR_KEYS.iter().enumerate() {
        assert_eq!(
            cellgov_mem::indexed_key(SUPERVISOR_KEY_SEED, k as u64),
            *key,
            "key {k}"
        );
    }
}

#[test]
fn supervisor_keys_are_distinct_nonzero_and_apart_from_the_lane_keys() {
    let distinct: BTreeSet<u128> = SUPERVISOR_KEYS.iter().copied().collect();
    assert_eq!(distinct.len(), SUPERVISOR_KEYS.len(), "two keys are equal");
    assert!(SUPERVISOR_KEYS.iter().all(|&k| k != 0), "a key is zero");
    assert!(
        KEYS.iter().all(|k| !distinct.contains(k)),
        "a supervisor key is also a lane key"
    );
}

#[test]
fn no_two_supervisor_multiplier_keys_cancel_modulo_2_pow_65() {
    let mask = (1u128 << 65) - 1;
    let mults = &SUPERVISOR_KEYS[1..];
    for (i, &a) in mults.iter().enumerate() {
        for (j, &b) in mults.iter().enumerate() {
            assert_ne!(
                a.wrapping_add(b) & mask,
                0,
                "words {i} and {j}: key sum is 0 mod 2^65"
            );
            if i != j {
                assert_ne!(
                    a.wrapping_sub(b) & mask,
                    0,
                    "words {i} and {j}: key difference is 0 mod 2^65"
                );
            }
        }
    }
}

/// A partition with every field away from [`Partition::INITIAL`]: a
/// 2^13-group page table at 16 MB, real mode over 256 MB at 256 MB, and
/// large pages of 64 KB (LP = 0) and 1 MB (LP = 1).
fn counting_partition() -> Partition {
    Partition::new(0x0100_0002, 1 << 28, 1 << 28, 0b1001).expect("a valid partition")
}

/// The block whose register and SLB words, in [`SupervisorState::words`]
/// order, are 1 through 139, with [`counting_partition`] after them.
fn counting_block() -> SupervisorState {
    let mut s = SupervisorState::new(1);
    s.srr0 = 2;
    s.srr1 = 3;
    s.dar = 4;
    s.dsisr = 5;
    s.dec = 6;
    s.dec_tb = 7;
    s.sprg = [8, 9, 10, 11];
    for (k, e) in s.slb.iter_mut().enumerate() {
        e.esid_v = 12 + 2 * k as u64;
        e.vsid_flags = 13 + 2 * k as u64;
    }
    s.partition = counting_partition();
    s
}

#[test]
fn supervisor_lane_wire_format_golden() {
    let mut counting: [u64; SUPERVISOR_WORDS] = std::array::from_fn(|i| i as u64 + 1);
    counting[SUPERVISOR_WORDS - 4..].copy_from_slice(&[0x0100_0002, 1 << 28, 1 << 28, 0b1001]);
    assert_eq!(
        counting_block().words(),
        counting,
        "the word order is part of the hash scheme"
    );
    assert_eq!(
        SupervisorState::new(0).digest(),
        0x71C5_E3B6_0FC3_EFA7,
        "written by hand in the same commit as the change that moves it"
    );
    assert_eq!(
        counting_block().digest(),
        0xB2F6_D3E3_4E1A_948F,
        "written by hand in the same commit as the change that moves it"
    );
    let mut state = PpuState::new();
    state.set_supervisor(Some(counting_block()));
    assert_eq!(
        state.state_hash(),
        0x3FB6_7FDD_BBEF_58C8,
        "written by hand in the same commit as the change that moves it"
    );
}

/// A named supervisor field, its width in bits, and how to flip one of
/// its bits.
type FieldFlip = (String, u32, Box<dyn Fn(&mut SupervisorState, u32)>);

fn field(
    name: String,
    width: u32,
    flip: impl Fn(&mut SupervisorState, u32) + 'static,
) -> FieldFlip {
    (name, width, Box::new(flip))
}

/// Every supervisor register, each SLB entry as its two halves.
fn field_flips() -> Vec<FieldFlip> {
    let mut out = vec![
        field("msr".into(), 64, |s, b| s.msr ^= 1u64 << b),
        field("srr0".into(), 64, |s, b| s.srr0 ^= 1u64 << b),
        field("srr1".into(), 64, |s, b| s.srr1 ^= 1u64 << b),
        field("dar".into(), 64, |s, b| s.dar ^= 1u64 << b),
        field("dsisr".into(), 32, |s, b| s.dsisr ^= 1u32 << b),
        field("dec".into(), 32, |s, b| s.dec ^= 1u32 << b),
        field("dec_tb".into(), 64, |s, b| s.dec_tb ^= 1u64 << b),
    ];
    for k in 0..4 {
        out.push(field(format!("sprg{k}"), 64, move |s, b| {
            s.sprg[k] ^= 1u64 << b;
        }));
    }
    for k in 0..PPE_SLB_ENTRIES {
        out.push(field(format!("slb{k}.esid_v"), 64, move |s, b| {
            s.slb[k].esid_v ^= 1u64 << b;
        }));
        out.push(field(format!("slb{k}.vsid_flags"), 64, move |s, b| {
            s.slb[k].vsid_flags ^= 1u64 << b;
        }));
    }
    out
}

#[test]
fn every_bit_of_every_supervisor_field_moves_the_state_hash() {
    let mut state = PpuState::new();
    state.set_supervisor(Some(SupervisorState::new(0)));
    let base = state.state_hash();
    let mut hashes = vec![base];
    for (name, width, flip) in field_flips() {
        for bit in 0..width {
            let mut flipped = state.clone();
            flipped
                .update_supervisor(|s| flip(s, bit))
                .expect("the state holds supervisor registers");
            let h = flipped.state_hash();
            assert_ne!(h, base, "{name} bit {bit}");
            assert!(flipped.hash_is_current(), "{name} bit {bit}");
            assert_eq!(h, hash(&lanes(&flipped.fingerprint())), "{name} bit {bit}");
            hashes.push(h);
        }
    }
    // Nine 64-bit registers, DSISR and DEC, and two words per SLB entry.
    assert_eq!(hashes.len(), 1 + 9 * 64 + 2 * 32 + 2 * 64 * PPE_SLB_ENTRIES);
    let n = hashes.len();
    hashes.sort_unstable();
    hashes.dedup();
    assert_eq!(
        hashes.len(),
        n,
        "two single-bit supervisor states share a hash"
    );
}

#[test]
fn each_partition_field_moves_the_state_hash() {
    let initial = Partition::INITIAL;
    let lb = initial.lb();
    let changed = [
        (
            "sdr1",
            Partition::new(0x0100_0000, initial.rm_limit(), 0, lb),
        ),
        ("rm_limit", Partition::new(0, 1 << 28, 0, lb)),
        ("rm_offset", Partition::new(0, 1 << 28, 1 << 28, lb)),
        ("lb", Partition::new(0, initial.rm_limit(), 0, 0b1001)),
    ];
    let mut state = PpuState::new();
    state.set_supervisor(Some(SupervisorState::new(0)));
    let base = state.state_hash();
    let mut seen = vec![base];
    for (name, partition) in changed {
        let partition = partition.expect("a valid partition");
        let mut moved = state.clone();
        moved
            .update_supervisor(|s| s.partition = partition)
            .expect("the state holds supervisor registers");
        assert!(moved.hash_is_current(), "{name}");
        assert_ne!(moved.state_hash(), base, "{name}");
        seen.push(moved.state_hash());
    }
    seen.sort_unstable();
    seen.dedup();
    assert_eq!(seen.len(), 5, "two partitions share a hash");
}
