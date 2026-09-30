//! The FPSCR: fscrwr / fscrrd round trip through the defined bits, the
//! rounding fields, per-slice accumulation, and sticky flags.

use cellgov_event::UnitId;
use cellgov_float::{Flags, Rounding};
use cellgov_ps3_abi::hw::spu_fpscr::{fpscr_field, FPSCR_DEFINED, FPSCR_RN_FIRST};

use crate::exec::{execute, SpuStepOutcome};
use crate::state::SpuState;

// [SPU-ISA p:235 s:9] fscrwr: RR opcode 0x3BA, RA the source.
// [SPU-ISA p:236 s:9] fscrrd: RR opcode 0x398, RT the destination.
const FSCRWR: u32 = 0x3BA << 21;
const FSCRRD: u32 = 0x398 << 21;

fn step(s: &mut SpuState, raw: u32) {
    let insn = crate::decode::decode(raw).expect("decodes");
    assert_eq!(execute(&insn, s, UnitId::new(0)), SpuStepOutcome::Continue);
}

#[test]
fn a_new_context_starts_with_a_zero_fpscr() {
    assert_eq!(SpuState::new().fpscr, 0);
}

// [SPU-ISA p:235 s:9] the unused bits fscrwr writes are undefined.
// [SPU-ISA p:236 s:9] fscrrd reads every unused bit as zero.
#[test]
fn writing_all_ones_reads_back_exactly_the_defined_bits() {
    let mut s = SpuState::new();
    s.regs[5] = [0xFF; 16];
    s.regs[9] = [0xAA; 16];
    step(&mut s, FSCRWR | 5 << 7 | 9);
    assert_eq!(s.regs[9], [0xAA; 16], "RT is a false target");
    assert_eq!(s.fpscr, FPSCR_DEFINED, "only the defined bits are stored");
    step(&mut s, FSCRRD | 3);
    assert_eq!(u128::from_be_bytes(s.regs[3]), FPSCR_DEFINED);
}

// [SPU-ISA p:200 s:9.3] 00 nearest even, 01 toward zero, 10 toward +infinity, 11 toward -infinity.
#[test]
fn the_rounding_fields_decode_all_four_modes_for_both_slices() {
    let modes = [
        Rounding::NearestEven,
        Rounding::TowardZero,
        Rounding::TowardPositive,
        Rounding::TowardNegative,
    ];
    for (slice, first) in FPSCR_RN_FIRST.into_iter().enumerate() {
        for (code, mode) in modes.into_iter().enumerate() {
            let mut s = SpuState::new();
            s.fpscr = (code as u128) << (128 - first - 2);
            let mut want = [Rounding::NearestEven; 2];
            want[slice] = mode;
            assert_eq!(s.fpscr_rounding(), want, "slice {slice} code {code}");
        }
    }
}

/// Flags with only `slice` set to `set`, the rest clear.
fn only<const N: usize>(slice: usize, set: Flags) -> [Flags; N] {
    let mut flags = [Flags::default(); N];
    flags[slice] = set;
    flags
}

// [SPU-ISA p:200 s:9.3] and [SPU-ISA p:201 s:9.3]: each slice has its own flag bits.
#[test]
fn accumulating_one_slice_leaves_every_other_bit_unchanged() {
    let all = Flags {
        overflow: true,
        underflow: true,
        inexact: true,
        invalid: true,
        nan: true,
        denormal: true,
        diff: true,
    };
    for (slice, first) in [29, 61, 93, 125].into_iter().enumerate() {
        let mut s = SpuState::new();
        s.fpscr_accumulate_single(only(slice, all));
        assert_eq!(s.fpscr, fpscr_field(first, 3), "single slice {slice}");
    }
    for (slice, first) in [50, 82].into_iter().enumerate() {
        let mut s = SpuState::new();
        s.fpscr_accumulate_double(only(slice, all));
        assert_eq!(s.fpscr, fpscr_field(first, 6), "double slice {slice}");
    }
    for slice in 0..4 {
        let mut s = SpuState::new();
        let mut divided = [false; 4];
        divided[slice] = true;
        s.fpscr_accumulate_dbz(divided);
        assert_eq!(
            s.fpscr,
            fpscr_field(116 + slice as u32, 1),
            "dbz slice {slice}"
        );
    }
    // Each flag lands on its own bit, in the order the ISA lists.
    let mut s = SpuState::new();
    s.fpscr_accumulate_single(only(
        1,
        Flags {
            underflow: true,
            ..Flags::default()
        },
    ));
    assert_eq!(s.fpscr, fpscr_field(62, 1));
    let mut s = SpuState::new();
    s.fpscr_accumulate_double(only(
        0,
        Flags {
            denormal: true,
            ..Flags::default()
        },
    ));
    assert_eq!(s.fpscr, fpscr_field(55, 1));
}

// [SPU-ISA p:200 s:9.3] every status bit stays set until fscrwr clears it.
#[test]
fn flags_stay_set_across_an_operation_that_raises_none() {
    let mut s = SpuState::new();
    s.fpscr_accumulate_single(only(
        2,
        Flags {
            overflow: true,
            ..Flags::default()
        },
    ));
    let before = s.fpscr;
    s.fpscr_accumulate_single([Flags::default(); 4]);
    s.fpscr_accumulate_double([Flags::default(); 2]);
    s.fpscr_accumulate_dbz([false; 4]);
    assert_eq!(s.fpscr, before);
    s.regs[1] = [0; 16];
    step(&mut s, FSCRWR | 1 << 7);
    assert_eq!(s.fpscr, 0, "fscrwr clears the sticky bits");
}

#[test]
fn the_fpscr_is_compared_and_only_fscrwr_may_change_it() {
    use crate::observation::{SpuAllowedFootprint, SpuObservation, SpuObservationComponent};
    use crate::state::SpuObservableSnapshot;

    let before = SpuState::new();
    let mut after = before.clone();
    after.fpscr = FPSCR_DEFINED;
    let observe = |state: &SpuState| {
        SpuObservation::from_parts(
            SpuObservableSnapshot::capture(state),
            SpuStepOutcome::Continue,
        )
    };
    assert!(observe(&before)
        .compare(&observe(&after))
        .differences
        .contains(&SpuObservationComponent::Fpscr));
    let fscrwr = crate::decode::decode(FSCRWR | 1 << 7).expect("decodes");
    let fscrrd = crate::decode::decode(FSCRRD | 3).expect("decodes");
    assert!(SpuAllowedFootprint::for_instruction(&fscrwr)
        .violations(&before, &observe(&after))
        .is_empty());
    assert!(SpuAllowedFootprint::for_instruction(&fscrrd)
        .violations(&before, &observe(&after))
        .contains(&SpuObservationComponent::Fpscr));
}
