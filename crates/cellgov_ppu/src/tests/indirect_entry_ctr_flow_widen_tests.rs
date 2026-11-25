use cellgov_ps3_abi::hw::ppc_isa::PPC_BLR;

use super::*;
use crate::instruction::encode::encode;

const BASE: u64 = 0x8000_0000_0020_0000;

/// The number of worker routines in [`code`].
const WORKERS: usize = 24;

/// The word of the first worker.
const FIRST_WORKER: usize = 7;

/// The words of one worker.
const WORKER_WORDS: usize = 6;

/// The displacements of the `lbz` of the four readers, in order.
///
/// - -174 is the slot of the copy at the `blr` of the routine at word 0.
/// - -32766 is the deepest slot of that width and residue that a
///   displacement reaches.
/// - -173 is a slot of another residue.
/// - 2 is a slot of the same width and residue above r1.
const READS: [i16; 4] = [-174, -32766, -173, 2];

fn word(instruction: PpuInstruction) -> u32 {
    encode(&instruction).expect("the instruction encodes")
}

fn address(word: usize) -> u64 {
    BASE + 4 * u64::try_from(word).expect("small")
}

/// `bl` at word `from` to word 0.
fn bl_routine(from: usize) -> u32 {
    word(PpuInstruction::B {
        offset: -i32::try_from(4 * from).expect("in reach"),
        aa: false,
        link: true,
    })
}

/// `bdnz` at word `from` to word `to`.
fn bdnz(from: usize, to: usize) -> u32 {
    let from = i16::try_from(from).expect("small");
    let to = i16::try_from(to).expect("small");
    word(PpuInstruction::Bc {
        bo: 0b10000,
        bi: 0,
        offset: 4 * (to - from),
        aa: false,
        link: false,
    })
}

fn addi_r1(imm: i16) -> u32 {
    word(PpuInstruction::Addi { rt: 1, ra: 1, imm })
}

fn stdu_r1(imm: i16) -> u32 {
    word(PpuInstruction::Stdu { rs: 1, ra: 1, imm })
}

/// The frame size of worker `index`.
fn frame(index: usize) -> i16 {
    112 + 16 * i16::try_from(index).expect("small")
}

/// This code at [`BASE`]:
/// - a routine at word 0: `stdu r1,-320(r1)`, `mtctr r9`, then a loop of
///   `mfctr r0`, `stb r0,146(r1)` and `bdnz`, then `addi r1,r1,320` and
///   `blr`;
/// - [`WORKERS`] routines: `stdu r1,-F(r1)`, `mtctr r9`, a `bdnz` to
///   itself, `bl` to word 0, `addi r1,r1,F`, `blr`, with a different `F`
///   for each worker;
/// - one reader for each displacement `D` of [`READS`]: `bl` to word 0,
///   `lbz r6,D(r1)`, `mtctr r6`, `bctr`.
///
/// Each return of a worker moves the returned slots deeper by its frame
/// size.
fn code() -> Code {
    let mut words = vec![
        stdu_r1(-320),
        word(PpuInstruction::Mtctr { rs: 9 }),
        word(PpuInstruction::Mfctr { rt: 0 }),
        word(PpuInstruction::Stb {
            rs: 0,
            ra: 1,
            imm: 146,
        }),
        bdnz(4, 2),
        addi_r1(320),
        PPC_BLR,
    ];
    assert_eq!(words.len(), FIRST_WORKER);
    for index in 0..WORKERS {
        let at = words.len();
        words.extend([
            stdu_r1(-frame(index)),
            word(PpuInstruction::Mtctr { rs: 9 }),
            bdnz(at + 2, at + 2),
            bl_routine(at + 3),
            addi_r1(frame(index)),
            PPC_BLR,
        ]);
        assert_eq!(words.len(), at + WORKER_WORDS);
    }
    for imm in READS {
        let at = words.len();
        words.extend([
            bl_routine(at),
            word(PpuInstruction::Lbz { rt: 6, ra: 1, imm }),
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

/// The word of the `bctr` of the reader of `READS[index]`.
fn reader_bctr(index: usize) -> usize {
    FIRST_WORKER + WORKERS * WORKER_WORDS + 4 * index + 3
}

/// Gives `check` the CTR flow over [`code`] after its run, and the number
/// of words that the run stepped. Each `mtctr r9` writes a lost CTR.
fn run(check: impl FnOnce(&CtrFlow<'_>, usize)) {
    let code = code();
    let lost: BTreeSet<(usize, u64)> = std::iter::once(1)
        .chain((0..WORKERS).map(|index| FIRST_WORKER + WORKER_WORDS * index + 1))
        .map(|word| (0, address(word)))
        .collect();
    assert_eq!(lost.len(), WORKERS + 1);
    let (constants, ranged) = (BTreeMap::new(), BTreeSet::new());
    let reach = Reach::every(&code);
    let writes = CtrWrites {
        constants: &constants,
        lost: &lost,
        ranged: &ranged,
    };
    let mut flow = CtrFlow::new(&code, writes, &reach);
    let stepped = flow.run();
    check(&flow, stepped);
}

/// [PPC-Book1 p:24 s:2.4] A branch with LK = 1 sets LR to CIA + 4.
/// [PPC-Book1 p:25 s:2.4] `bclr` branches to LR 0:61 || 0b00.
/// [PPC-Book1 p:51 s:3.3.8] `addi` and `addis` add EXTS(SI) or EXTS(SI || 0x0000) to RA, or to 0 when RA is 0.
/// [PPC-Book1 p:40 s:3.3.3] A store with update places EA into RA.
#[test]
fn lost_loop_count_copies_that_returns_move_through_24_frames_settle_in_two_steps_a_word() {
    run(|flow, stepped| {
        let words: usize = flow.at.iter().map(Vec::len).sum();
        assert_eq!(
            words,
            FIRST_WORKER + WORKERS * WORKER_WORDS + 4 * READS.len()
        );
        assert!(stepped >= words, "{stepped} steps over {words} words");
        assert!(stepped <= 2 * words, "{stepped} steps over {words} words");
        let after_call = &flow.at[0][FIRST_WORKER + 4].slots.lost;
        assert!(after_call.contains((-174, 1)));
        assert!(after_call.contains((-32766, 1)));
    });
}

/// [PPC-Book1 p:25 s:2.4] `bclr` reaches LR0:61 || 0b00, and `bcctr` reaches CTR0:61 || 0b00.
/// [PPC-Book1 p:82 s:3.3.13] `mfctr`, a form of `mfspr`, copies CTR into RT.
#[test]
fn a_copy_from_the_returned_or_deepest_widened_slot_reaches_its_bctr_lost_and_other_slots_do_not() {
    run(|flow, _| {
        let lost = flow.bcctr_targets(&mut BTreeMap::new());
        assert_eq!(
            lost,
            BTreeSet::from([address(reader_bctr(0)), address(reader_bctr(1))])
        );
    });
}

#[test]
fn a_widened_slot_set_keeps_both_sets_and_takes_each_end_that_grows_to_the_reach() {
    let reach = -64..64;
    let held = SlotSet::of([(-14, 1), (2, 1)]);
    let other_residue = held.widened(&SlotSet::of([(-13, 1)]), &reach);
    assert_eq!(other_residue, SlotSet::of([(-14, 1), (2, 1), (-13, 1)]));
    let other_width = held.widened(&SlotSet::of([(-14, 2)]), &reach);
    assert_eq!(other_width, SlotSet::of([(-14, 1), (2, 1), (-14, 2)]));
    assert_eq!(held.widened(&SlotSet::of([(2, 1)]), &reach), held);
    let between = SlotSet::of([(-30, 1), (34, 1)]).widened(&SlotSet::of([(2, 1)]), &reach);
    assert_eq!(
        between,
        SlotSet::of([(-30, 1), (-14, 1), (2, 1), (18, 1), (34, 1)])
    );
    let deeper = held.widened(&SlotSet::of([(-30, 1)]), &reach);
    assert_eq!(
        deeper,
        SlotSet::of([(-62, 1), (-46, 1), (-30, 1), (-14, 1), (2, 1)])
    );
    let higher = deeper.widened(&SlotSet::of([(18, 1)]), &reach);
    assert_eq!(higher.len(), 8);
    assert!(higher.contains((-62, 1)) && higher.contains((50, 1)));
    assert_eq!(
        higher.widened(&SlotSet::of([(34, 1), (-46, 1)]), &reach),
        higher
    );
    let to_start = SlotSet::of([(0, 4)]).widened(&SlotSet::of([(-16, 4)]), &reach);
    assert_eq!(
        to_start,
        SlotSet::of([(-64, 4), (-48, 4), (-32, 4), (-16, 4), (0, 4)])
    );
}

/// [PPC-Book1 p:46 s:3.3.5] `stmw` stores consecutive words from EA up, so a part can lie past the highest displacement.
#[test]
fn a_slot_past_the_end_of_the_reach_stays_in_the_widened_set_and_grows_its_class_again() {
    let reach = -64..64;
    let at_reach = SlotSet::of([(16, 4)]).widened(&SlotSet::of([(32, 4)]), &reach);
    assert_eq!(at_reach, SlotSet::of([(16, 4), (32, 4), (48, 4)]));
    let past = at_reach.widened(&SlotSet::of([(64, 4)]), &reach);
    assert_ne!(past, at_reach);
    assert!(past.contains((64, 4)));
    assert_eq!(past.len(), 4);
}
