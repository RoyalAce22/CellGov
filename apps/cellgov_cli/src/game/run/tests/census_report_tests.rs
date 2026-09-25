use super::*;

#[test]
fn the_census_block_names_each_count_and_each_sample() {
    let mut lanes = [0u64; cellgov_ppu::multilinear::LANE_COUNT];
    lanes[0] = 0x10;
    lanes[37] = 0x80;
    let report = CensusReport {
        dispatches: 9,
        distinct_states: 7,
        state_hash_collisions: 1,
        multilinear_collisions: 0,
        identity_conflicts: 0,
        samples: vec![(4, lanes)],
    };
    let lines = census_lines(&report);
    assert_eq!(
        lines[1..7],
        [
            "state-hash census:",
            "  dispatches:             9",
            "  distinct states:        7",
            "  state_hash collisions:  1",
            "  multilinear collisions: 0",
            "  identity conflicts:     0",
        ]
    );
    assert_eq!(lines.len(), 8);
    assert!(
        lines[7].starts_with("  sample 4: [0x10, 0x0, "),
        "{}",
        lines[7]
    );
    assert!(lines[7].ends_with(", 0x0, 0x80]"), "{}", lines[7]);
}
