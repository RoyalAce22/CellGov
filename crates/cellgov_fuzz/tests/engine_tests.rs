//! Integration tests for the public fuzz-engine API.

use cellgov_fuzz::{ppu, spu, CampaignSchedule, CaseRange, FuzzConfig};

fn small_config() -> FuzzConfig {
    FuzzConfig {
        seed: 7,
        schedule: CampaignSchedule {
            cases: CaseRange {
                first: 0,
                count: 16,
            },
            ..CampaignSchedule::default()
        },
        max_findings: 4,
        sequence_words: 4,
        ..FuzzConfig::default()
    }
}

#[test]
fn engines_are_deterministic_library_calls() {
    let config = small_config();
    assert_eq!(ppu::run_instructions(config), ppu::run_instructions(config));
    assert_eq!(ppu::run_sequences(config), ppu::run_sequences(config));
    assert_eq!(spu::run_instructions(config), spu::run_instructions(config));
    assert_eq!(spu::run_sequences(config), spu::run_sequences(config));
}

#[test]
fn concurrent_runs_keep_independent_results() {
    let config = small_config();
    let expected = ppu::run_instructions(config);
    let workers = (0..4)
        .map(|_| std::thread::spawn(move || ppu::run_instructions(config)))
        .collect::<Vec<_>>();

    for worker in workers {
        assert_eq!(worker.join().expect("worker must not panic"), expected);
    }
}
