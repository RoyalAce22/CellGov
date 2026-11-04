//! The cache-block instructions: icbi, dcbst and dcbf fault where a
//! load of the addressed byte faults, and the touch and data-stream
//! hints never fault. None of them changes state or emits an effect.

use crate::exec::test_support::uid;
use crate::exec::{execute, ExecuteVerdict};
use crate::instruction::encode::encode;
use crate::instruction::fuzz::{generation_descriptors, PpuOutcomeClass};
use crate::instruction::PpuInstruction as I;
use crate::state::PpuState;
use crate::store_buffer::StoreBuffer;

const PAGE_BASE: u64 = 0x1000;

const PROBES: [I; 3] = [
    I::Dcbst { ra: 3, rb: 4 },
    I::Dcbf { ra: 3, rb: 4, l: 0 },
    I::Icbi { ra: 3, rb: 4 },
];

const HINTS: [I; 5] = [
    I::Dcbt {
        ra: 3,
        rb: 4,
        th: 0,
    },
    I::Dcbtst {
        ra: 3,
        rb: 4,
        th: 0,
    },
    I::Dst {
        ra: 3,
        rb: 4,
        strm: 1,
        t: true,
    },
    I::Dstst {
        ra: 3,
        rb: 4,
        strm: 2,
        t: false,
    },
    I::Dss { strm: 3, all: true },
];

/// Runs `insn` with r3 = `PAGE_BASE` and r4 = `offset` over one mapped page,
/// and checks that it left no state, effect or store behind.
fn run(insn: I, offset: u64) -> ExecuteVerdict {
    let page = [0u8; 0x1000];
    let views = [cellgov_mem::RegionView::plain(PAGE_BASE, &page)];
    let mut state = PpuState::new();
    state.set_gpr(0, 0x10_0000);
    state.set_gpr(3, PAGE_BASE);
    state.set_gpr(4, offset);
    let hash = state.state_hash();
    let mut effects = Vec::new();
    let mut stores = StoreBuffer::new();
    let verdict = execute(&insn, &mut state, uid(), &views, &mut effects, &mut stores);
    assert_eq!(state.state_hash(), hash, "{insn:?} changed state");
    assert!(effects.is_empty(), "{insn:?} emitted {effects:?}");
    assert!(stores.is_empty(), "{insn:?} buffered a store");
    verdict
}

#[test]
fn a_cache_block_probe_of_a_mapped_byte_continues() {
    for insn in PROBES {
        for offset in [0, 0x7F, 0xFFF] {
            assert_eq!(
                run(insn, offset),
                ExecuteVerdict::Continue,
                "{insn:?} +0x{offset:x}"
            );
        }
    }
}

#[test]
fn a_cache_block_probe_of_an_unmapped_byte_faults_as_lbzx_does() {
    for offset in [0x1000u64, 0x1001, 0xFFFF_F000] {
        let lbzx = run(
            I::Lbzx {
                rt: 5,
                ra: 3,
                rb: 4,
            },
            offset,
        );
        assert!(matches!(lbzx, ExecuteVerdict::MemFault(_)), "{lbzx:?}");
        for insn in PROBES {
            assert_eq!(run(insn, offset), lbzx, "{insn:?} +0x{offset:x}");
        }
    }
}

/// [PPC-Book2 p:21 s:3.2.2] the effective address is (RA|0) + RB.
#[test]
fn a_cache_block_probe_with_ra_zero_addresses_rb_alone() {
    for insn in [
        I::Dcbst { ra: 0, rb: 4 },
        I::Dcbf { ra: 0, rb: 4, l: 0 },
        I::Icbi { ra: 0, rb: 4 },
    ] {
        assert_eq!(
            run(insn, PAGE_BASE + 8),
            ExecuteVerdict::Continue,
            "{insn:?}"
        );
    }
}

#[test]
fn each_cache_block_kind_has_a_fuzz_descriptor_that_admits_only_its_outcomes() {
    let generated: Vec<_> = generation_descriptors()
        .into_iter()
        .map(|d| d.kind)
        .collect();
    for (insns, outcomes) in [
        (
            &PROBES[..],
            &[PpuOutcomeClass::Continue, PpuOutcomeClass::MemoryFault][..],
        ),
        (&HINTS[..], &[PpuOutcomeClass::Continue][..]),
    ] {
        for &insn in insns {
            let raw = encode(&insn).expect("encodes");
            let descriptor = insn.fuzz_descriptor(raw);
            assert!(generated.contains(&descriptor.kind), "{insn:?}");
            assert_eq!(descriptor.outcomes, outcomes, "{insn:?}");
            assert!(descriptor.effects.is_empty(), "{insn:?}");
        }
    }
}

/// [PPC-Book2 p:18 s:3.2.1] icbi is treated as a Load, so a region that refuses stores still takes it.
#[test]
fn a_cache_block_probe_of_a_write_refused_byte_continues() {
    let page = [0u8; 0x1000];
    let views = [cellgov_mem::RegionView {
        write_refused_by: Some("text"),
        ..cellgov_mem::RegionView::plain(PAGE_BASE, &page)
    }];
    for insn in PROBES {
        let mut state = PpuState::new();
        state.set_gpr(3, PAGE_BASE);
        state.set_gpr(4, 0x40);
        let mut effects = Vec::new();
        let mut stores = StoreBuffer::new();
        let verdict = execute(&insn, &mut state, uid(), &views, &mut effects, &mut stores);
        assert_eq!(verdict, ExecuteVerdict::Continue, "{insn:?}");
    }
}

#[test]
fn a_touch_or_stream_hint_of_an_unmapped_byte_continues() {
    for insn in HINTS {
        assert_eq!(run(insn, 0x1000), ExecuteVerdict::Continue, "{insn:?}");
    }
}
