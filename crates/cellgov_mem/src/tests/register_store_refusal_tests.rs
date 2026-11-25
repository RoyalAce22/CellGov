//! The two refusals of a store to a register word: the device's refusal
//! and the second register store of one batch.

use super::*;
use crate::{GuestAddr, PageSize, Region, StagedWrite, StagingMemory};

/// A span below 2^32, so the zero-padded address form differs from the
/// unpadded one.
const SPAN: (u64, u64) = (0x8000, 0x1000);
const WORD: u64 = SPAN.0 + 0x10;
const NEXT: u64 = SPAN.0 + 0x14;

/// A rule that refuses a store with bit 0 set and keeps the stored word
/// otherwise.
fn refuse_bit_0(words: &mut [RegisterWord], index: usize, store: RegisterStore) -> bool {
    if store.value & 1 != 0 {
        return false;
    }
    words[index] = RegisterWord::known(store.value);
    true
}

fn memory() -> GuestMemory {
    let mut mem =
        GuestMemory::from_regions(vec![Region::new(0, 0x4000, "main", PageSize::Page4K)]).unwrap();
    mem.declare_unmodelled(SPAN.0, SPAN.1).unwrap();
    mem.declare_registers(
        &[(WORD, RegisterWord::UNKNOWN), (NEXT, RegisterWord::UNKNOWN)],
        refuse_bit_0,
    )
    .unwrap();
    mem
}

fn range(addr: u64) -> ByteRange {
    ByteRange::new(GuestAddr::new(addr), 4).unwrap()
}

/// Drains one batch of `stores` into `mem`, each as its word's address
/// and its stored value.
fn drain(mem: &mut GuestMemory, stores: &[(u64, u32)]) -> Result<usize, MemError> {
    let mut staging = StagingMemory::new();
    for &(addr, value) in stores {
        staging.stage(StagedWrite::new(range(addr), &value.to_be_bytes()));
    }
    let drained = staging.drain_into(mem);
    staging.clear();
    drained
}

/// Each producer of a register store refusal, with the error it gives.
fn refusals() -> Vec<(&'static str, MemError)> {
    let mut mem = memory();
    let refused = [0, 0, 0, 1];
    vec![
        (
            "validate_commit",
            mem.validate_commit(range(WORD), &refused).unwrap_err(),
        ),
        (
            "apply_staged",
            mem.apply_staged(range(WORD), &refused).unwrap_err(),
        ),
        (
            "one refused store",
            drain(&mut mem, &[(WORD, 1)]).unwrap_err(),
        ),
        (
            "two accepted stores",
            drain(&mut mem, &[(WORD, 2), (NEXT, 4)]).unwrap_err(),
        ),
    ]
}

#[test]
fn the_device_refusal_and_the_second_store_of_a_batch_give_their_own_variants() {
    let device = MemError::RegisterStoreRefused { real: WORD };
    let expected = [
        ("validate_commit", device),
        ("apply_staged", device),
        ("one refused store", device),
        (
            "two accepted stores",
            MemError::SecondRegisterStore { real: NEXT },
        ),
    ];
    assert_eq!(refusals(), expected);
}

#[test]
fn each_register_store_refusal_names_its_failure_and_the_padded_real_address() {
    let device = "the device refuses a store to the register word at real 0x0000000000008010";
    let second = "the store to the register word at real 0x0000000000008014 is the second \
                  register store of its batch";
    let expected = [
        ("validate_commit", device),
        ("apply_staged", device),
        ("one refused store", device),
        ("two accepted stores", second),
    ]
    .map(|(producer, message)| (producer, message.to_string()));
    let messages: Vec<(&str, String)> = refusals()
        .into_iter()
        .map(|(producer, error)| (producer, error.to_string()))
        .collect();
    assert_eq!(messages, expected);
}
