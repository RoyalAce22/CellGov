use super::PpeIssuePressure;

#[test]
fn the_larger_of_two_pressures_takes_each_field_from_either() {
    let a = PpeIssuePressure {
        cycles: 1,
        vsu_cycles: 4,
        multiply_cycles: 5,
        divide_cycles: 1,
        divide_fraction: 200,
        multiply_hold_share: 0,
        vscr_cycles: 2,
        drain_cycles: 1,
        queue_cycles: 6,
        fence_cycles: 0,
        kernel_share: 200,
        issue_interval: 700,
    };
    let b = PpeIssuePressure {
        cycles: 3,
        vsu_cycles: 2,
        multiply_cycles: 2,
        divide_cycles: 2,
        divide_fraction: 16,
        multiply_hold_share: 128,
        vscr_cycles: 9,
        drain_cycles: 4,
        queue_cycles: 1,
        fence_cycles: 3,
        kernel_share: 10,
        issue_interval: 300,
    };
    let want = PpeIssuePressure {
        cycles: 3,
        vsu_cycles: 4,
        multiply_cycles: 5,
        // The divide stall comes whole from b: 2 + 16/256 cycles, more than
        // a's 1 + 200/256.
        divide_cycles: 2,
        divide_fraction: 16,
        multiply_hold_share: 128,
        vscr_cycles: 9,
        drain_cycles: 4,
        queue_cycles: 6,
        fence_cycles: 3,
        kernel_share: 200,
        issue_interval: 700,
    };
    assert_eq!(a.max(b), want);
    assert_eq!(b.max(a), want);
    assert_eq!(a.max(PpeIssuePressure::NONE), a);
    // On the same whole cycles the larger fraction is the larger stall.
    let c = PpeIssuePressure {
        divide_fraction: 200,
        ..b
    };
    assert_eq!(b.max(c), c);
    assert_eq!(c.max(b), c);
}
