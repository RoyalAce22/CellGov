use super::PpeIssuePressure;

#[test]
fn the_larger_of_two_pressures_takes_each_field_from_either() {
    let a = PpeIssuePressure {
        cycles: 1,
        multiply_cycles: 5,
        multiply_hold_share: 0,
    };
    let b = PpeIssuePressure {
        cycles: 3,
        multiply_cycles: 2,
        multiply_hold_share: 128,
    };
    let want = PpeIssuePressure {
        cycles: 3,
        multiply_cycles: 5,
        multiply_hold_share: 128,
    };
    assert_eq!(a.max(b), want);
    assert_eq!(b.max(a), want);
    assert_eq!(a.max(PpeIssuePressure::NONE), a);
}
