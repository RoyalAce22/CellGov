//! A branch into the second slot of a fused pair runs the instruction
//! stored there, on every internal path.

use cellgov_fuzz::ppu_paths::{first_path_divergence, run_all_paths, PpuExecutionPath};
use cellgov_ppu::instruction::PpuInstruction;
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ppu::state::PpuState;

const DATA_BASE: u64 = 0x1000_0000;

fn lwz(rt: u32, ra: u32, d: u16) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(d)
}

fn ld(rt: u32, ra: u32, ds: u16) -> u32 {
    (58 << 26) | (rt << 21) | (ra << 16) | u32::from(ds & 0xfffc)
}

fn stw(rs: u32, ra: u32, d: u16) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(d)
}

fn std(rs: u32, ra: u32, ds: u16) -> u32 {
    (62 << 26) | (rs << 21) | (ra << 16) | u32::from(ds & 0xfffc)
}

fn li(rt: u32, value: u16) -> u32 {
    (14 << 26) | (rt << 21) | u32::from(value)
}

fn cmpwi(ra: u32, imm: u16) -> u32 {
    (11 << 26) | (ra << 16) | u32::from(imm)
}

fn cmpw(ra: u32, rb: u32) -> u32 {
    (31 << 26) | (ra << 16) | (rb << 11)
}

fn mflr(rt: u32) -> u32 {
    (31 << 26) | (rt << 21) | (8 << 16) | (339 << 1)
}

fn mtlr(rs: u32) -> u32 {
    (31 << 26) | (rs << 21) | (8 << 16) | (467 << 1)
}

/// `bc 20,0,bd`: branch always, relative.
fn bc_always(bd: u16) -> u32 {
    (16 << 26) | (20 << 21) | u32::from(bd & 0xfffc)
}

/// `b` to `offset` bytes past itself.
fn b(offset: u32) -> u32 {
    (18 << 26) | (offset & 0x03ff_fffc)
}

const BLR: u32 = 0x4e80_0020;
const BCTR: u32 = 0x4e80_0420;
const NOP: u32 = 24 << 26;

/// A state whose registers make each second half visible: r3 differs
/// from every value a first half would load or set, and r4 and r5
/// address the data region.
fn seeded_state() -> PpuState {
    let mut state = PpuState::new();
    state.set_gpr(3, 9);
    state.set_gpr(4, DATA_BASE);
    state.set_gpr(5, 0x0102_0304_0506_0708);
    state.set_lr(0x40);
    state
}

/// Data whose words differ from r3, so a load in a first half changes r3.
fn seeded_data() -> Vec<u8> {
    (0..64u8)
        .map(|i| i.wrapping_mul(17).wrapping_add(1))
        .collect()
}

fn shadow_of(words: &[u32]) -> PredecodedShadow {
    let code = words
        .iter()
        .flat_map(|word| word.to_be_bytes())
        .collect::<Vec<_>>();
    PredecodedShadow::build(0, &code)
}

/// Runs `words` on every path and asserts that the pair before `second`
/// fused and that no path differs from the plain one.
fn assert_entry_matches(words: &[u32], second: u64, state: &PpuState) {
    assert_eq!(
        shadow_of(words).get(second),
        Some(PpuInstruction::Consumed),
        "precondition: the pair before 0x{second:x} fuses in {words:08x?}"
    );
    let runs = run_all_paths(words, state, &seeded_data()).expect("entry case must run");
    assert_eq!(runs[3].path, PpuExecutionPath::Fused);
    assert!(
        first_path_divergence(&runs).is_none(),
        "{words:08x?}: {:#?}",
        first_path_divergence(&runs)
    );
}

#[test]
fn a_branch_into_the_second_slot_runs_it_for_every_fusion_kind() {
    let pairs: [(&str, [u32; 2]); 9] = [
        ("LwzCmpwi", [lwz(3, 4, 8), cmpwi(3, 5)]),
        ("LwzCmpwi from CmpwZero", [lwz(3, 4, 8), cmpwi(3, 0)]),
        ("LiStw", [li(3, 7), stw(3, 4, 0)]),
        ("MflrStw", [mflr(3), stw(3, 4, 0)]),
        ("LwzMtlr", [lwz(3, 4, 8), mtlr(3)]),
        ("MflrStd", [mflr(3), std(3, 4, 0)]),
        ("LdMtlr", [ld(3, 4, 8), mtlr(3)]),
        ("StdStd", [std(3, 4, 0), std(5, 4, 8)]),
        ("CmpwBc", [cmpw(3, 5), bc_always(8)]),
    ];
    for (kind, [first, second]) in pairs {
        let words = [b(8), first, second, li(6, 1), NOP];
        assert_eq!(
            shadow_of(&words).get(8),
            Some(PpuInstruction::Consumed),
            "precondition: {kind} fuses"
        );
        assert_entry_matches(&words, 8, &seeded_state());
    }
}

#[test]
fn a_branch_into_a_fused_compare_and_branch_takes_the_stored_branch() {
    let words = [b(8), cmpwi(3, 5), bc_always(8), li(6, 1), NOP];
    assert!(matches!(
        shadow_of(&words).get(4),
        Some(PpuInstruction::CmpwiBc { .. })
    ));

    assert_entry_matches(&words, 8, &seeded_state());
    let runs = run_all_paths(&words, &seeded_state(), &seeded_data()).expect("case must run");
    assert!(runs.iter().all(|run| run.observation.state.gpr[6] == 0));
}

#[test]
fn a_branch_into_the_second_slot_stores_what_the_plain_path_stores() {
    let words = [b(8), li(3, 7), stw(3, 4, 0)];

    assert_entry_matches(&words, 8, &seeded_state());
    let runs = run_all_paths(&words, &seeded_state(), &seeded_data()).expect("case must run");
    assert!(runs
        .iter()
        .all(|run| run.observation.memory[..4] == 9u32.to_be_bytes()));
}

#[test]
fn a_return_into_the_second_slot_runs_it() {
    let words = [BLR, li(3, 7), stw(3, 4, 0)];
    let mut state = seeded_state();
    state.set_lr(8);

    assert_entry_matches(&words, 8, &state);
    let runs = run_all_paths(&words, &state, &seeded_data()).expect("case must run");
    assert!(runs
        .iter()
        .all(|run| run.observation.memory[..4] == 9u32.to_be_bytes()));
}

#[test]
fn a_count_register_jump_into_the_second_slot_runs_it() {
    let words = [BCTR, li(3, 7), stw(3, 4, 0)];
    let mut state = seeded_state();
    state.set_ctr(8);

    assert_entry_matches(&words, 8, &state);
    let runs = run_all_paths(&words, &state, &seeded_data()).expect("case must run");
    assert!(runs
        .iter()
        .all(|run| run.observation.memory[..4] == 9u32.to_be_bytes()));
}

/// SplitMix64: a fixed-seed generator, so every run checks the same
/// cases.
struct Cases(u64);

impl Cases {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d4_9bb8_2213_3111);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
}

#[test]
fn random_branch_targets_over_fusable_code_match_the_plain_path() {
    const CASES: u64 = 256;
    const BODY: usize = 8;
    let pairs: [[u32; 2]; 8] = [
        [lwz(3, 4, 8), cmpwi(3, 5)],
        [li(3, 7), stw(3, 4, 0)],
        [mflr(3), stw(3, 4, 4)],
        [lwz(3, 4, 12), mtlr(3)],
        [mflr(3), std(3, 4, 16)],
        [ld(3, 4, 24), mtlr(3)],
        [std(3, 4, 32), std(5, 4, 40)],
        [cmpw(3, 5), bc_always(8)],
    ];
    let mut cases = Cases(0x1979);
    let mut second_slot_entries = 0;
    for case in 0..CASES {
        let mut words = vec![0; 1];
        for _ in 0..BODY / 2 {
            words.extend_from_slice(&pairs[cases.below(pairs.len() as u64) as usize]);
        }
        let target = 4 + 4 * cases.below(BODY as u64) as u32;
        words[0] = b(target);
        if shadow_of(&words).get(u64::from(target)) == Some(PpuInstruction::Consumed) {
            second_slot_entries += 1;
        }
        let runs = run_all_paths(&words, &seeded_state(), &seeded_data())
            .unwrap_or_else(|e| panic!("case {case} ({words:08x?}) must run: {e}"));
        assert!(
            first_path_divergence(&runs).is_none(),
            "case {case} ({words:08x?}): {:#?}",
            first_path_divergence(&runs)
        );
    }
    assert!(
        second_slot_entries >= CASES / 4,
        "only {second_slot_entries} of {CASES} cases branched into a second slot"
    );
}
