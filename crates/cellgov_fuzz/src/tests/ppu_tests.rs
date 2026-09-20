use super::*;

fn mftb(rt: u32) -> u32 {
    (31 << 26) | (rt << 21) | (12 << 16) | (8 << 11) | (371 << 1)
}

#[test]
fn seeded_vrsave_is_valid_for_an_immediate_read() {
    let mut rng = Rng::for_iter(7, 0);
    let initial = random_state(&mut rng);
    let observed = run_once(
        &PpuInstruction::Mfvrsave { rt: 0 },
        &initial,
        &[0; DATA_LEN],
    );

    assert_eq!(observed.state.gpr[0], u64::from(initial.vrsave));
}

#[test]
fn a_time_base_read_is_part_of_the_observed_effects() {
    let initial = PpuState::new();
    let instruction = cellgov_ppu::decode::decode(mftb(3)).unwrap();
    let observed = run_once(&instruction, &initial, &[0; DATA_LEN]);

    assert!(observed
        .effects
        .iter()
        .any(|effect| matches!(effect, Effect::ClockRead { source } if *source == UNIT)));
}

#[test]
fn a_ppu_sequence_retains_terminal_effects() {
    let initial = PpuState::new();
    let (observed, decoded, _) = run_sequence(&[mftb(3)], &initial, &[0; DATA_LEN]);

    assert_eq!(decoded, 1);
    assert!(matches!(
        observed.terminal_verdict,
        Some(ExecuteVerdict::Continue)
    ));
    assert!(observed
        .effects
        .iter()
        .any(|effect| matches!(effect, Effect::ClockRead { source } if *source == UNIT)));
}

#[test]
fn a_ppu_sequence_fetches_from_the_branch_target() {
    let branch_over_next_word = (18 << 26) | 8;
    let li_r3_one = (14 << 26) | (3 << 21) | 1;
    let nop = 24 << 26;
    let initial = PpuState::new();

    let (observed, decoded, _) = run_sequence(
        &[branch_over_next_word, li_r3_one, nop],
        &initial,
        &[0; DATA_LEN],
    );

    assert_eq!(decoded, 2);
    assert_eq!(observed.state.gpr[3], 0);
}

#[test]
fn a_faulting_ppu_sequence_discards_prior_state() {
    let li_r3_one = (14 << 26) | (3 << 21) | 1;
    let lwz_r4_r5 = (32 << 26) | (4 << 21) | (5 << 16);
    let initial = PpuState::new();

    let (observed, decoded, _) = run_sequence(&[li_r3_one, lwz_r4_r5], &initial, &[0; DATA_LEN]);

    assert_eq!(decoded, 2);
    assert_eq!(observed.state.gpr[3], 0);
    assert_eq!(observed.pc, 0);
    assert!(observed.effects.is_empty());
    assert!(matches!(
        observed.terminal_verdict,
        Some(ExecuteVerdict::MemFault(_))
    ));
}

#[test]
fn a_zero_word_ppu_sequence_is_an_invalid_configuration() {
    let report = run_sequences(FuzzConfig {
        iterations: 1,
        sequence_words: 0,
        ..FuzzConfig::default()
    });

    assert_eq!(
        report
            .finding_counts
            .get(&FindingKind::InvalidConfiguration),
        Some(&1)
    );
    assert_eq!(report.cases, 0);
}

#[test]
fn replay_requires_the_deterministic_relation() {
    assert!(!requests_replay(&[]));
    assert!(requests_replay(&[PpuMetamorphicRelation::Deterministic]));
}
