//! Model-based properties of the store buffer against a last-write-wins byte map.

use std::collections::BTreeMap;

use proptest::prelude::*;

use super::*;

/// Base of the clustered address space, so generated stores overlap often.
const BASE: u64 = 0x1000;
/// Width of the clustered address space in bytes.
const SPAN: u8 = 48;
/// Byte the overlay window holds before the buffer patches it.
const FILL: u8 = 0xAA;

/// One operation the driver applies to the buffer and the `Oracle` alike.
#[derive(Debug, Clone)]
enum Op {
    Insert { off: u8, len: u8, value: u128 },
    InsertConditional { off: u8, len: u8, value: u128 },
    Forward { off: u8, len: u8 },
    Overlay { off: u8, len: u8 },
    HasStore { off: u8, len: u8 },
    Flush,
    Clear,
}

fn op() -> impl Strategy<Value = Op> {
    let off = 0..SPAN;
    prop_oneof![
        4 => (off.clone(), 1u8..=16, any::<u128>())
            .prop_map(|(off, len, value)| Op::Insert { off, len, value }),
        1 => (off.clone(), 1u8..=16, any::<u128>())
            .prop_map(|(off, len, value)| Op::InsertConditional { off, len, value }),
        3 => (off.clone(), 1u8..=16).prop_map(|(off, len)| Op::Forward { off, len }),
        2 => (off.clone(), 1u8..=32).prop_map(|(off, len)| Op::Overlay { off, len }),
        1 => (off, 1u8..=32).prop_map(|(off, len)| Op::HasStore { off, len }),
        1 => Just(Op::Flush),
        1 => Just(Op::Clear),
    ]
}

/// Last-write-wins bytes, plus the program-order record that `flush` must emit.
#[derive(Default)]
struct Oracle {
    bytes: BTreeMap<u64, u8>,
    entries: Vec<(u64, u8, u128, bool)>,
}

impl Oracle {
    fn write(&mut self, addr: u64, len: u8, value: u128, conditional: bool) {
        let all = value.to_be_bytes();
        for i in 0..len {
            self.bytes.insert(
                addr + u64::from(i),
                all[16 - usize::from(len) + usize::from(i)],
            );
        }
        self.entries.push((addr, len, value, conditional));
    }

    /// The bytes of `[addr, addr + len)`, right-aligned, or `None` when
    /// the byte map lacks one of them.
    fn window(&self, addr: u64, len: u8) -> Option<u128> {
        let mut out = [0u8; 16];
        for i in 0..len {
            out[16 - usize::from(len) + usize::from(i)] =
                *self.bytes.get(&(addr + u64::from(i)))?;
        }
        Some(u128::from_be_bytes(out))
    }

    /// Whether the most recent entry that touches the window covers it whole.
    fn single_entry_covers(&self, addr: u64, len: u8) -> bool {
        let end = addr + u64::from(len);
        self.entries
            .iter()
            .rev()
            .find(|(a, l, _, _)| *a < end && a + u64::from(*l) > addr)
            .is_some_and(|(a, l, _, _)| *a <= addr && a + u64::from(*l) >= end)
    }

    fn clear(&mut self) {
        self.bytes.clear();
        self.entries.clear();
    }
}

fn effect_record(effect: &Effect) -> Option<(u64, u8, Vec<u8>, bool)> {
    match effect {
        Effect::SharedWriteIntent { range, bytes, .. } => Some((
            range.start().raw(),
            range.length() as u8,
            bytes.bytes().to_vec(),
            false,
        )),
        Effect::ConditionalStore { range, bytes, .. } => Some((
            range.start().raw(),
            range.length() as u8,
            bytes.bytes().to_vec(),
            true,
        )),
        _ => None,
    }
}

proptest! {
    #[test]
    fn the_buffer_agrees_with_a_last_write_wins_byte_map(ops in prop::collection::vec(op(), 1..80)) {
        let mut buf = StoreBuffer::new();
        let mut oracle = Oracle::default();
        for op in ops {
            match op {
                Op::Insert { off, len, value } => {
                    let addr = BASE + u64::from(off);
                    let staged = buf.insert(addr, len, value);
                    if oracle.entries.len() < CAPACITY {
                        prop_assert_eq!(staged, Ok(()));
                        oracle.write(addr, len, value, false);
                    } else {
                        prop_assert_eq!(staged, Err(StoreRefusal::Full));
                    }
                }
                Op::InsertConditional { off, len, value } => {
                    let addr = BASE + u64::from(off);
                    let staged = buf.insert_conditional(addr, len, value, 0);
                    if oracle.entries.len() < CAPACITY {
                        prop_assert_eq!(staged, Ok(()));
                        oracle.write(addr, len, value, true);
                    } else {
                        prop_assert_eq!(staged, Err(StoreRefusal::Full));
                    }
                }
                Op::Forward { off, len } => {
                    let addr = BASE + u64::from(off);
                    let forwarded = buf.forward(addr, len);
                    if oracle.single_entry_covers(addr, len) {
                        prop_assert_eq!(forwarded, oracle.window(addr, len));
                    } else {
                        prop_assert_eq!(forwarded, None);
                    }
                }
                Op::Overlay { off, len } => {
                    let base = BASE + u64::from(off);
                    let mut out = vec![FILL; usize::from(len)];
                    buf.overlay_range(base, &mut out);
                    let expected: Vec<u8> = (0..u64::from(len))
                        .map(|i| oracle.bytes.get(&(base + i)).copied().unwrap_or(FILL))
                        .collect();
                    prop_assert_eq!(out, expected);
                }
                Op::HasStore { off, len } => {
                    let base = BASE + u64::from(off);
                    let end = base + u64::from(len);
                    let expected = oracle.bytes.range(base..end).next().is_some();
                    prop_assert_eq!(buf.has_store_in_range(base, end), expected);
                }
                Op::Flush => {
                    let mut effects = Vec::new();
                    buf.flush(&mut effects, UnitId::new(0));
                    let emitted: Vec<_> = effects.iter().filter_map(effect_record).collect();
                    let expected: Vec<_> = oracle
                        .entries
                        .iter()
                        .map(|&(addr, len, value, conditional)| {
                            let all = value.to_be_bytes();
                            (addr, len, all[16 - usize::from(len)..].to_vec(), conditional)
                        })
                        .collect();
                    prop_assert_eq!(emitted, expected);
                    prop_assert!(buf.is_empty());
                    oracle.clear();
                }
                Op::Clear => {
                    buf.clear();
                    prop_assert!(buf.is_empty());
                    oracle.clear();
                }
            }
            prop_assert_eq!(buf.len(), oracle.entries.len());
        }
    }

    #[test]
    fn a_store_whose_range_wraps_the_address_space_is_refused_by_name(
        below_max in 0u64..24,
        len in 1u8..=16,
        conditional in any::<bool>(),
    ) {
        let addr = u64::MAX - below_max;
        let mut buf = StoreBuffer::new();
        let staged = if conditional {
            buf.insert_conditional(addr, len, 0x55, 0)
        } else {
            buf.insert(addr, len, 0x55)
        };
        if addr.checked_add(u64::from(len)).is_some() {
            prop_assert_eq!(staged, Ok(()));
            prop_assert_eq!(buf.len(), 1);
            prop_assert_eq!(buf.forward(addr, len), Some(0x55));
        } else {
            prop_assert_eq!(staged, Err(StoreRefusal::AddressWraps { addr, len }));
            prop_assert!(buf.is_empty());
        }
    }

    #[test]
    fn a_load_whose_range_wraps_the_address_space_is_never_forwarded(
        below_max in 0u64..24,
        len in 1u8..=16,
    ) {
        let addr = u64::MAX - below_max;
        let mut buf = StoreBuffer::new();
        let lower = u64::MAX - 32;
        let upper = u64::MAX - 16;
        let lower_bytes = u128::from_be_bytes([0x22; 16]);
        let upper_bytes = u128::from_be_bytes([0x44; 16]);
        buf.insert(lower, 16, lower_bytes).expect("a store inside the space stages");
        buf.insert(upper, 16, upper_bytes).expect("a store ending one short of the top stages");
        let forwarded = buf.forward(addr, len);
        match addr.checked_add(u64::from(len)) {
            Some(end) => {
                // `end` never exceeds the top of the space, so the upper
                // entry covers every window that starts inside it.
                let covered = (addr >= lower && end <= upper) || addr >= upper;
                prop_assert_eq!(forwarded.is_some(), covered);
            }
            None => prop_assert_eq!(forwarded, None),
        }
        // The window is clipped at the top of the space, and the last
        // byte of the space belongs to no entry.
        let mut out = [FILL; 8];
        buf.overlay_range(addr, &mut out);
        for (i, byte) in out.iter().enumerate() {
            let expected = match addr.checked_add(i as u64) {
                Some(p) if p < u64::MAX => if p >= upper { 0x44 } else { 0x22 },
                _ => FILL,
            };
            prop_assert_eq!(*byte, expected, "byte {} of the window at 0x{:016x}", i, addr);
        }
        prop_assert_eq!(buf.has_store_in_range(addr, u64::MAX), addr < u64::MAX);
    }
}
