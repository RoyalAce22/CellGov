//! The store-conditional results the architecture leaves undefined, held
//! to the `ppu_reservation_edges` capture from the reference console.
//!
//! [PPC-Book2 p:25 s:3.3.2] A store-conditional to an address other than
//! the reserved one leaves the store and CR0[EQ] undefined.
//! [PPC-Book2 p:10 s:1.7.3.1] Whether the holder's own `dcbf`, or another
//! processor's `dcbtst`, `dcbst` or `dcbf`, on the reserved granule loses
//! the reservation is undefined; the holder's own `dcbst` is not in the
//! list of events that lose it. Every case below gave one answer in all
//! of its console trials, and CellGov gives it.

use super::*;
use cellgov_effects::WritePayload;

const BASE: u64 = 0x1000;
const VALUE: u64 = 0x1122_3344_5566_7700;

/// The probe's payload, as big-endian u32 words.
fn words() -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro/ppu_reservation_edges/ps3/cech20-cex-493/cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "frame magic");
    let len = u32::from_be_bytes(frame[4..8].try_into().expect("4 bytes")) as usize;
    assert_eq!(
        (len, frame.len() - 8),
        (4 * (3 + 3 * 18), len),
        "frame length"
    );
    frame[8..]
        .chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4 bytes")))
        .collect()
}

#[derive(Clone, Copy, Debug)]
enum Reserve {
    Word,
    Doubleword,
}

#[derive(Clone, Copy, Debug)]
enum Between {
    Nothing,
    Dcbf,
    Dcbst,
}

/// One single-thread case of `ppu/main.c`: the load-reserve on line + 0,
/// the cache operation between, and the store-conditional at line + off.
struct Pair {
    reserve: Reserve,
    between: Between,
    store: Reserve,
    off: u64,
}

const fn pair(reserve: Reserve, store: Reserve, off: u64) -> Pair {
    Pair {
        reserve,
        between: Between::Nothing,
        store,
        off,
    }
}

const SINGLE: [Pair; 14] = [
    pair(Reserve::Word, Reserve::Word, 0),
    pair(Reserve::Word, Reserve::Word, 4),
    pair(Reserve::Word, Reserve::Word, 64),
    pair(Reserve::Word, Reserve::Word, 124),
    pair(Reserve::Word, Reserve::Word, 128),
    pair(Reserve::Doubleword, Reserve::Doubleword, 0),
    pair(Reserve::Doubleword, Reserve::Doubleword, 8),
    pair(Reserve::Doubleword, Reserve::Doubleword, 64),
    pair(Reserve::Doubleword, Reserve::Doubleword, 120),
    pair(Reserve::Doubleword, Reserve::Doubleword, 128),
    pair(Reserve::Word, Reserve::Doubleword, 0),
    pair(Reserve::Doubleword, Reserve::Word, 0),
    Pair {
        between: Between::Dcbf,
        ..pair(Reserve::Word, Reserve::Word, 0)
    },
    Pair {
        between: Between::Dcbst,
        ..pair(Reserve::Word, Reserve::Word, 0)
    },
];

/// Runs `insns` in order over a 256-byte buffer at `BASE`, flushes the
/// store buffer, and returns every effect.
fn run(insns: &[PpuInstruction], state: &mut PpuState) -> Vec<Effect> {
    let mem = [0xA5u8; 0x100];
    let views = [cellgov_mem::RegionView::plain(BASE, &mem)];
    let mut effects = Vec::new();
    let mut store_buf = StoreBuffer::new();
    for insn in insns {
        let v = execute(insn, state, uid(), &views, &mut effects, &mut store_buf);
        assert_eq!(v, ExecuteVerdict::Continue, "{insn:?}");
    }
    store_buf.flush(&mut effects, uid());
    effects
}

/// CR0[EQ] after the case, and whether its store-conditional wrote VALUE
/// to line + off.
fn cellgov(p: &Pair) -> (bool, bool) {
    let mut s = PpuState::new();
    s.set_gpr(3, BASE);
    s.set_gpr(4, 0);
    s.set_gpr(5, BASE + p.off);
    s.set_gpr(6, VALUE);
    let mut insns = vec![match p.reserve {
        Reserve::Word => PpuInstruction::Lwarx {
            rt: 7,
            ra: 3,
            rb: 4,
        },
        Reserve::Doubleword => PpuInstruction::Ldarx {
            rt: 7,
            ra: 3,
            rb: 4,
        },
    }];
    match p.between {
        Between::Nothing => {}
        Between::Dcbf => insns.push(PpuInstruction::Dcbf { ra: 3, rb: 4, l: 0 }),
        Between::Dcbst => insns.push(PpuInstruction::Dcbst { ra: 3, rb: 4 }),
    }
    let (store, bytes) = match p.store {
        Reserve::Word => (
            PpuInstruction::Stwcx {
                rs: 6,
                ra: 5,
                rb: 4,
            },
            (VALUE as u32).to_be_bytes().to_vec(),
        ),
        Reserve::Doubleword => (
            PpuInstruction::Stdcx {
                rs: 6,
                ra: 5,
                rb: 4,
            },
            VALUE.to_be_bytes().to_vec(),
        ),
    };
    insns.push(store);
    let effects = run(&insns, &mut s);
    let stored = effects.iter().any(|e| {
        matches!(e, Effect::ConditionalStore { range, bytes: b, .. }
            if range.start().raw() == BASE + p.off && *b == WritePayload::new(bytes.clone()))
    });
    (s.cr_field(0) & 0b0010 != 0, stored)
}

/// The console's count for an answer CellGov gives every time.
fn all_or_none(answer: bool, trials: u32) -> u32 {
    if answer {
        trials
    } else {
        0
    }
}

#[test]
fn every_single_thread_case_matches_the_console() {
    let w = words();
    assert_eq!((w[0], w[1]), (0, 0), "no step failed");
    let trials = w[2];
    assert_eq!(trials, 32);
    for (c, p) in SINGLE.iter().enumerate() {
        let (eq, stored) = cellgov(p);
        let console = (w[3 + 3 * c], w[4 + 3 * c], w[5 + 3 * c]);
        assert_eq!(
            console,
            (all_or_none(eq, trials), all_or_none(stored, trials), 0),
            "case {c}: {:?} {:?} {:?} +{}",
            p.reserve,
            p.between,
            p.store,
            p.off
        );
    }
}

/// CellGov clears another unit's reservation only at commit, from an
/// effect; `dcbst`, `dcbf` and `dcbtst` emit no effect.
#[test]
fn another_threads_cache_operation_keeps_the_reservation_as_on_the_console() {
    let w = words();
    let trials = w[2];
    for (c, op) in [
        (15, PpuInstruction::Dcbst { ra: 3, rb: 4 }),
        (16, PpuInstruction::Dcbf { ra: 3, rb: 4, l: 0 }),
        (
            17,
            PpuInstruction::Dcbtst {
                ra: 3,
                rb: 4,
                th: 0,
            },
        ),
    ] {
        let console = (w[3 + 3 * c], w[4 + 3 * c], w[5 + 3 * c]);
        assert_eq!(console, (trials, trials, 0), "case {c}: {op:?}");
        let mut other = PpuState::new();
        other.set_gpr(3, BASE);
        other.set_gpr(4, 0);
        let effects = run(&[op], &mut other);
        assert!(effects.is_empty(), "case {c}: {op:?} emits: {effects:?}");
    }
}
