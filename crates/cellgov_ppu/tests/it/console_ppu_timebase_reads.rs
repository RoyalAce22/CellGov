//! CellGov's time-base reads against the `ppu_timebase_reads` console
//! capture.
//!
//! The probe has three parts:
//!
//! - it reads `mftb` back to back;
//! - it runs `mftbu` / `mftb` / `mftbu` triples across a carry out of
//!   the low word;
//! - two threads publish a time-base value to each other.
//!
//! The replays run each part through the PPU's issue model.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::{ExecutionContext, ExecutionUnit};
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::{Budget, GuestTicks, TICKS_PER_TB_TICK};

const CONSOLE: &str = "../../tests/micro/ppu_timebase_reads/ps3/cech20-cex-493/observation.json";

/// The payload's word 89 when the probe wrote every word.
const DONE: u64 = 0x5050_5442;

/// The console's payload as big-endian words.
fn console_words() -> Vec<u64> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 720);
    let words: Vec<u64> = p
        .chunks_exact(8)
        .map(|w| u64::from_be_bytes(w.try_into().expect("8 bytes")))
        .collect();
    assert_eq!(words[89], DONE, "the probe wrote every word");
    assert_eq!(words[77], 0, "the second thread was created");
    assert_eq!(words[78], 1, "the second thread ran");
    assert_eq!(words[85], 0, "the second thread joined");
    assert_eq!(
        words[86], 0x5442,
        "the second thread exited through its own path"
    );
    words
}

/// `mftb rt`.
const fn mftb(rt: u32) -> u32 {
    0x7c0c_42e6 | rt << 21
}

/// `addi rt, ra, v`; `li rt, v` when `ra` is 0.
const fn addi(rt: u32, ra: u32, v: i16) -> u32 {
    0x3800_0000 | rt << 21 | ra << 16 | (v as u16 as u32)
}

/// A `bc` with BO and BI `bo_bi`, from word `from` to word `to`.
fn bc(bo_bi: u32, from: usize, to: usize) -> u32 {
    0x4000_0000 | bo_bi << 16 | ((4 * (to as i64 - from as i64)) as u32 & 0xfffc)
}

const BNE: u32 = 0x0082;
const BGE: u32 = 0x0080;
const BDNZ: u32 = 0x0200;
const LWSYNC: u32 = 0x7c20_04ac;
/// `b .`: where a replayed program ends.
const STAY: u32 = 0x4800_0000;

fn place(mem: &mut GuestMemory, at: u64, words: &[u32]) {
    for (i, w) in words.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(at + 4 * i as u64), 4).expect("range");
        mem.apply_commit(range, &w.to_be_bytes()).expect("mapped");
    }
}

fn ppu(rt: &Runtime, id: UnitId) -> &PpuExecutionUnit {
    rt.registry()
        .get(id)
        .and_then(|u| u.as_any().downcast_ref::<PpuExecutionUnit>())
        .expect("a PPU unit")
}

/// Run `rt` until every unit in `units` sits on its closing `b .`.
fn run_to_end(rt: &mut Runtime, units: &[(UnitId, u64)], max_steps: usize) {
    for _ in 0..max_steps {
        if units.iter().all(|&(id, end)| ppu(rt, id).state().pc == end) {
            return;
        }
        let mut step = rt.step().expect("the replay steps");
        rt.commit_step(&mut step).expect("the step commits");
    }
    panic!("the replay did not end within {max_steps} steps");
}

const CODE: u64 = 0x1000;

/// The deltas of eight back-to-back `mftb` reads after `lead`
/// independent `addi`, which set the reads' phase within a tick.
fn back_to_back_deltas(lead: usize) -> Vec<u64> {
    let mut words: Vec<u32> = (0..lead).map(|_| addi(20, 20, 1)).collect();
    words.extend((3..11).map(mftb));
    words.push(STAY);
    let end = CODE + 4 * (words.len() as u64 - 1);
    let mut mem = GuestMemory::new(0x4000);
    place(&mut mem, CODE, &words);
    let mut rt = Runtime::new(mem, Budget::new(256), usize::MAX);
    let id = rt.register_unit_with(|id| {
        let mut ppu = PpuExecutionUnit::new(id);
        ppu.state_mut().pc = CODE;
        ppu
    });
    run_to_end(&mut rt, &[(id, end)], 64);
    let gpr = &ppu(&rt, id).state().gpr;
    (3..10).map(|r| gpr[r + 1] - gpr[r]).collect()
}

#[test]
fn back_to_back_reads_step_as_the_console_s_do() {
    let w = console_words();
    // The probe's last run: delta j lies between reads j and j + 1, and a
    // group's stores sit between reads 8g + 7 and 8g + 8.
    let console: Vec<u64> = (0..63).filter(|j| j % 8 != 7).map(|j| w[j]).collect();
    assert_eq!(console.len(), 56);
    assert_eq!(w[63], 0, "no two console reads repeated");
    // Word 64: the smallest delta across every console run.
    assert_eq!(
        w[64], 1,
        "the console's back-to-back reads step by one tick"
    );
    for lead in 0..48 {
        let deltas = back_to_back_deltas(lead);
        assert_eq!(
            deltas.iter().min(),
            Some(&w[64]),
            "lead {lead}: CellGov's smallest delta, of {deltas:?}"
        );
        for d in deltas {
            assert!(
                console.contains(&d),
                "lead {lead}: CellGov's delta {d}, the console's {console:?}"
            );
        }
    }
}

/// `mftbu rt`.
const fn mftbu(rt: u32) -> u32 {
    0x7c0d_42e6 | rt << 21
}

/// `cmpld ra, rb` into CR0.
const fn cmpld(ra: u32, rb: u32) -> u32 {
    0x7c20_0040 | ra << 16 | rb << 11
}

/// `rldicl ra, rs, sh, 32`: `srdi ra, rs, 32` at `sh` 32, `clrldi ra, rs,
/// 32` at `sh` 0.
const fn rldicl_32(ra: u32, rs: u32, sh: u32) -> u32 {
    0x7800_0020 | rs << 21 | ra << 16 | (sh & 0x1f) << 11 | (sh >> 5) << 1
}

const BEQ: u32 = 0x0182;

/// Triples the carry replay runs.
const TRIPLES: i16 = 96;

/// The probe's part B across a carry, as guest words.
///
/// The program reads `mftbu`, `mftb`, `mftbu` into r3, r4, r5 `TRIPLES`
/// times. It counts:
///
/// - in r20, the inconsistent triples;
/// - in r21, the triples whose second `mftbu` is the first plus one;
/// - in r22, the carries out of the `mftb` low word.
fn carry_triples() -> Vec<u32> {
    let mut w = vec![
        addi(20, 0, 0),
        addi(21, 0, 0),
        addi(22, 0, 0),
        addi(6, 0, 0),
        addi(9, 0, TRIPLES),
        0x7d29_03a6, // mtctr r9
    ];
    let top = w.len();
    w.extend([mftbu(3), mftb(4), mftbu(5), addi(7, 3, 1), cmpld(5, 7)]);
    let at = w.len();
    w.push(bc(BNE, at, at + 2));
    w.push(addi(21, 21, 1));
    // Consistent: equal mftbu reads naming the mftb's high word, or the
    // second one more than the first.
    w.push(cmpld(3, 5));
    let differ = w.len() + 5;
    let mono = differ + 4;
    let bad = differ + 2;
    let carry = mono + 3;
    let at = w.len();
    w.push(bc(BNE, at, differ));
    w.push(rldicl_32(8, 4, 32));
    w.push(cmpld(8, 3));
    let at = w.len();
    w.push(bc(BEQ, at, mono));
    let at = w.len();
    w.push(0x4800_0000 | (4 * (bad - at)) as u32); // b bad
    assert_eq!(w.len(), differ);
    w.push(cmpld(5, 7));
    w.push(bc(BEQ, differ + 1, mono));
    assert_eq!(w.len(), bad);
    w.push(addi(20, 20, 1));
    w.push(0x4800_0000 | (4 * (carry - (bad + 1))) as u32); // b carry
    assert_eq!(w.len(), mono);
    // An mftb below the triple before is inconsistent too.
    w.push(cmpld(4, 6));
    w.push(bc(BGE, mono + 1, carry));
    w.push(addi(20, 20, 1));
    assert_eq!(w.len(), carry);
    w.push(rldicl_32(8, 4, 0));
    w.push(rldicl_32(9, 6, 0));
    w.push(cmpld(8, 9));
    let at = w.len();
    w.push(bc(BGE, at, at + 2));
    w.push(addi(22, 22, 1));
    w.push(0x7c86_2378); // mr r6, r4
    let at = w.len();
    w.push(bc(BDNZ, at, top));
    w.push(STAY);
    w
}

/// The carry replay's (inconsistent, plus one, carries) counts, run as one
/// step from guest tick `tick`.
fn triples_across_a_carry(tick: u64) -> (u64, u64, u64) {
    let words = carry_triples();
    let mut mem = GuestMemory::new(0x4000);
    place(&mut mem, CODE, &words);
    let mut ppu = PpuExecutionUnit::new(UnitId::new(0));
    ppu.state_mut().pc = CODE;
    let ctx = ExecutionContext::new(&mem).with_current_tick(GuestTicks::new(tick));
    ppu.run_until_yield(Budget::new(64 * 1024), &ctx, &mut Vec::new());
    let s = ppu.state();
    assert_eq!(
        s.pc,
        CODE + 4 * (words.len() as u64 - 1),
        "the replay ran every triple"
    );
    (s.gpr[20], s.gpr[21], s.gpr[22])
}

#[test]
fn mftbu_mftb_mftbu_triples_across_a_low_word_carry_stay_consistent_as_the_console_s_do() {
    let w = console_words();
    assert_eq!(w[74], 1, "the console's low word carried");
    // Whether the console's own carry falls inside a triple changes from
    // run to run; at most one of its triples can straddle it.
    assert!(w[72] <= 1, "the console straddled {} triples", w[72]);
    // The first triple reads a hundred time-base ticks before the carry,
    // at each phase within a tick.
    let start = (0x1_0000_0000 - 100) * TICKS_PER_TB_TICK;
    let mut straddled = 0;
    for phase in 0..TICKS_PER_TB_TICK {
        let (bad, plus_one, carried) = triples_across_a_carry(start + phase);
        assert_eq!(bad, w[73], "phase {phase}: inconsistent triples");
        assert_eq!(carried, w[74], "phase {phase}: carries");
        // Whether a triple straddles the carry depends on where the carry
        // falls in the loop; at most one can.
        assert!(plus_one <= 1, "phase {phase}: {plus_one} straddled");
        straddled += plus_one;
    }
    assert!(straddled > 0, "some phase puts the carry inside a triple");
}

/// Base of the words the handshake shares.
///
/// - +0: the published value;
/// - +0x80: the publisher's sequence;
/// - +0x100: the reader's sequence.
const SHARED: u64 = 0x3000;
const READER: u64 = 0x1800;
const ROUNDS: i16 = 64;

/// The publisher's program, which starts with `lead` independent `addi`.
///
/// Each of its `ROUNDS` rounds:
///
/// 1. reads `mftb` and stores it;
/// 2. runs `lwsync`;
/// 3. stores the round's sequence;
/// 4. waits for the reader to echo the sequence.
fn publisher(lead: usize) -> Vec<u32> {
    let mut w: Vec<u32> = (0..lead).map(|_| addi(20, 20, 1)).collect();
    w.push(addi(12, 0, 1));
    w.push(addi(9, 0, ROUNDS));
    w.push(0x7d29_03a6); // mtctr r9
    let top = w.len();
    w.push(mftb(3));
    w.push(0xf86a_0000); // std r3, 0(r10)
    w.push(LWSYNC);
    w.push(0x918a_0080); // stw r12, 0x80(r10)
    let wait = w.len();
    w.push(0x808a_0100); // lwz r4, 0x100(r10)
    w.push(0x7c04_6000); // cmpw r4, r12
    w.push(bc(BNE, w.len(), wait));
    w.push(addi(12, 12, 2));
    w.push(bc(BDNZ, w.len(), top));
    w.push(STAY);
    w
}

/// The reader's program.
///
/// Each of its `ROUNDS` rounds:
///
/// 1. waits for the round's sequence;
/// 2. runs `lwsync` and loads the published value;
/// 3. reads `mftb`, and counts in r20 a read below the published value;
/// 4. echoes the sequence.
fn reader() -> Vec<u32> {
    let mut w = vec![
        addi(12, 0, 1),
        addi(20, 0, 0),
        addi(9, 0, ROUNDS),
        0x7d29_03a6, // mtctr r9
    ];
    let top = w.len();
    w.push(0x808a_0080); // lwz r4, 0x80(r10)
    w.push(0x7c04_6000); // cmpw r4, r12
    w.push(bc(BNE, w.len(), top));
    w.push(LWSYNC);
    w.push(0xe8aa_0000); // ld r5, 0(r10)
    w.push(mftb(6));
    w.push(0x7c26_2840); // cmpld r6, r5
    let skip = w.len() + 2;
    w.push(bc(BGE, w.len(), skip));
    w.push(addi(20, 20, 1));
    w.push(0x918a_0100); // stw r12, 0x100(r10)
    w.push(addi(12, 12, 2));
    w.push(bc(BDNZ, w.len(), top));
    w.push(STAY);
    w
}

/// The reader's reads below the published value over every round, for a
/// publisher lead of `lead` instructions and a budget of `budget`
/// instructions a step.
fn reads_below_published(lead: usize, budget: u64) -> u64 {
    let (p, r) = (publisher(lead), reader());
    let mut mem = GuestMemory::new(0x4000);
    place(&mut mem, CODE, &p);
    place(&mut mem, READER, &r);
    let mut rt = Runtime::new(mem, Budget::new(budget), usize::MAX);
    let start = |at: u64| {
        move |id| {
            let mut ppu = PpuExecutionUnit::new(id);
            let s = ppu.state_mut();
            s.pc = at;
            s.set_gpr(10, SHARED);
            ppu
        }
    };
    let a = rt.register_unit_with(start(CODE));
    let b = rt.register_unit_with(start(READER));
    let ends = [
        (a, CODE + 4 * (p.len() as u64 - 1)),
        (b, READER + 4 * (r.len() as u64 - 1)),
    ];
    run_to_end(&mut rt, &ends, 400_000);
    ppu(&rt, b).state().gpr[20]
}

#[test]
fn a_read_after_another_thread_s_published_read_is_never_below_it() {
    let w = console_words();
    assert_eq!((w[79], w[82]), (1000, 1000), "every console round ran");
    assert_eq!((w[80], w[83]), (0, 0), "no console read fell below");
    for budget in [1, 7, 64, 256] {
        for lead in 0..12 {
            assert_eq!(
                reads_below_published(lead, budget),
                w[80],
                "lead {lead}, budget {budget}",
            );
        }
    }
}
