use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

use super::*;
use crate::instruction::encode::encode;

const BASE: u64 = 0x8000_0000_0010_0000;

/// The number of routines that call the routine at word 0, then add 16
/// to r1.
const CALLERS: usize = 8;

/// The word of the `bctr` whose CTR comes from the deepest slot that the
/// returns fill.
const DEEP_BCTR: usize = 4 + 3 * CALLERS + 3;

/// The word of the `bctr` whose CTR comes from a slot that no return
/// fills.
const OTHER_BCTR: usize = DEEP_BCTR + 4;

fn word(instruction: PpuInstruction) -> u32 {
    encode(&instruction).expect("the instruction encodes")
}

/// `bl` at word `from` to word 0.
fn bl_routine(from: usize) -> u32 {
    let offset = -i32::try_from(4 * from).expect("in reach");
    word(PpuInstruction::B {
        offset,
        aa: false,
        link: true,
    })
}

/// Gives `flow` the CTR flow over `code` after its run, with a lost CTR
/// from the `mtctr` at word 0.
fn run(code: &Code, flow: impl FnOnce(&CtrFlow<'_>)) {
    let writes = BTreeMap::new();
    let lost_writes = BTreeSet::from([(0, BASE)]);
    let mut ctr_flow = CtrFlow::new(code, &writes, &lost_writes);
    ctr_flow.run();
    flow(&ctr_flow);
}

/// This code at [`BASE`]:
/// - a routine at word 0: `mtctr r3`, `mfctr r5`, `std r5,8(r1)`, `blr`;
/// - [`CALLERS`] routines: `bl` to word 0, `addi r1,r1,16`, `blr`;
/// - `bl` to word 0, `ld r6,-32760(r1)`, `mtctr r6`, `bctr`;
/// - `bl` to word 0, `ld r6,0(r1)`, `mtctr r6`, `bctr`.
///
/// Each return of a caller brings the returned slots 16 bytes deeper. So
/// the returns fill the slots from 8(r1) down to -32760(r1) in steps of
/// 16 bytes. -32760(r1) is the deepest of these that a displacement
/// reaches.
fn code() -> Code {
    let mut words = vec![
        word(PpuInstruction::Mtctr { rs: 3 }),
        word(PpuInstruction::Mfctr { rt: 5 }),
        word(PpuInstruction::Std {
            rs: 5,
            ra: 1,
            imm: 8,
        }),
        PPC_BLR,
    ];
    for _ in 0..CALLERS {
        let at = words.len();
        words.extend([
            bl_routine(at),
            word(PpuInstruction::Addi {
                rt: 1,
                ra: 1,
                imm: 16,
            }),
            PPC_BLR,
        ]);
    }
    for imm in [-32760, 0] {
        let at = words.len();
        words.extend([
            bl_routine(at),
            word(PpuInstruction::Ld { rt: 6, ra: 1, imm }),
            word(PpuInstruction::Mtctr { rs: 6 }),
            word(PpuInstruction::Bcctr {
                bo: 0b10100,
                bi: 0,
                link: false,
            }),
        ]);
    }
    Code::new(vec![CodeSegment { vaddr: BASE, words }])
}

/// The map entries, mask bits and slot runs that `state` keeps. The word
/// of `mtctr r6` keeps three: the lost copies in r5 and r6, and one run
/// of slots.
fn records(state: &CtrState) -> usize {
    let gprs = usize::try_from(state.copies.lost.count_ones()).expect("at most 32");
    state.ctr.known.len()
        + state.copies.known.len()
        + gprs
        + state.slots.known.len()
        + state.slots.lost.run_count()
}

/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:25 s:2.4] `bclr` branches to LR 0:61 || 0b00.
/// [PPC-Book1 p:51 s:3.3.8] `addi` and `addis` add EXTS(SI) or EXTS(SI || 0x0000) to RA, or to 0 when RA is 0.
#[test]
fn lost_copies_in_2049_returned_slots_take_at_most_three_records_a_word() {
    run(&code(), |flow| {
        let states = || flow.at.iter().flatten();
        assert_eq!(
            states().map(|state| state.slots.lost.len()).max(),
            Some(2049)
        );
        assert_eq!(states().map(records).max(), Some(3));
    });
}

/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
#[test]
fn a_ctr_from_the_deepest_returned_slot_is_lost_and_one_from_an_unfilled_slot_is_not() {
    run(&code(), |flow| {
        let lost = flow.bcctr_targets(&mut BTreeMap::new());
        let address = |word: usize| BASE + 4 * u64::try_from(word).expect("small");
        let at_other_mtctr = &flow.at[0][OTHER_BCTR - 1].copies;
        assert_eq!(at_other_mtctr.get(5), Some(Ctr::lost()));
        assert_eq!(at_other_mtctr.get(6), None);
        assert_eq!(lost, BTreeSet::from([address(DEEP_BCTR)]));
    });
}
