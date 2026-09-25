//! These runtime fields enter the sync-state hash:
//!
//! - the DMA queue
//! - the pending DMA tag bits
//! - the deferred RSX effects
//! - the pending child inits
//! - the seeded RSX label base

use cellgov_dma::{DmaCompletion, DmaDirection, DmaRequest};
use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_time::{Budget, GuestTicks};

use crate::runtime::spaces::AddressSpaceId;
use crate::runtime::state::Runtime;
use crate::runtime::types::PendingChildInit;

fn fresh() -> Runtime {
    Runtime::new(GuestMemory::new(16), Budget::new(4), 100)
}

/// Each case sets one field of a fresh runtime from `k`; `k` = 1 and
/// `k` = 2 differ only in that field's value.
#[test]
fn every_pending_runtime_field_moves_the_hash_alone() {
    fn dma(rt: &mut Runtime, time: u64) {
        let range = |start| ByteRange::new(GuestAddr::new(start), 0x10).unwrap();
        let request = DmaRequest::new(
            DmaDirection::Put,
            range(0x100),
            range(0x200),
            UnitId::new(1),
        )
        .unwrap();
        rt.dma_queue
            .enqueue(DmaCompletion::new(request, GuestTicks::new(time)), None);
    }
    fn child(rt: &mut Runtime, k: u64, field: usize) {
        let pick = |i: usize, base: u64| if i == field { base + k } else { base };
        rt.pending_child_inits.push(PendingChildInit {
            pid: pick(0, 2) as u32,
            space: AddressSpaceId::new(pick(1, 1) as u32),
            primary_unit: UnitId::new(pick(2, 5)),
            init_token: pick(3, 9),
        });
    }
    type Set = fn(&mut Runtime, u64);
    let cases: [(&str, Set); 10] = [
        ("dma queue", dma),
        ("dma tag completion", |rt, k| {
            rt.pending_tag_completions.insert(UnitId::new(1), 1 << k);
        }),
        ("rsx label write offset", |rt, k| {
            rt.pending_rsx_effects.push(Effect::RsxLabelWrite {
                offset: k as u32,
                value: 1,
            });
        }),
        ("rsx label write value", |rt, k| {
            rt.pending_rsx_effects.push(Effect::RsxLabelWrite {
                offset: 0x10,
                value: k as u32,
            });
        }),
        ("rsx flip request", |rt, k| {
            rt.pending_rsx_effects.push(Effect::RsxFlipRequest {
                buffer_index: k as u8,
            });
        }),
        ("child init pid", |rt, k| child(rt, k, 0)),
        ("child init space", |rt, k| child(rt, k, 1)),
        ("child init unit", |rt, k| child(rt, k, 2)),
        ("child init token", |rt, k| child(rt, k, 3)),
        ("rsx label base", |rt, k| {
            rt.rsx_label_base = k as u32 * 0x1000
        }),
    ];
    let empty = fresh().sync_state_hash();
    for (what, set) in cases {
        let hashes = [1, 2].map(|k| {
            let mut rt = fresh();
            set(&mut rt, k);
            let hash = rt.sync_state_hash();
            assert_eq!(hash, rt.sync_state_hash_from_scratch(), "{what} = {k}");
            hash
        });
        assert_ne!(hashes[0], empty, "{what} did not move the hash");
        assert_ne!(hashes[0], hashes[1], "{what}'s value did not move the hash");
    }
}

#[test]
fn pending_lists_hash_their_order() {
    let build = |effects: [Effect; 2], inits: [u64; 2]| {
        let mut rt = fresh();
        rt.pending_rsx_effects.extend(effects);
        rt.pending_child_inits
            .extend(inits.map(|init_token| PendingChildInit {
                pid: 2,
                space: AddressSpaceId::new(1),
                primary_unit: UnitId::new(5),
                init_token,
            }));
        rt.sync_state_hash()
    };
    let label = |value| Effect::RsxLabelWrite { offset: 0, value };
    let base = build([label(1), label(2)], [7, 8]);
    assert_ne!(base, build([label(2), label(1)], [7, 8]), "effect order");
    assert_ne!(base, build([label(1), label(2)], [8, 7]), "init order");
}
