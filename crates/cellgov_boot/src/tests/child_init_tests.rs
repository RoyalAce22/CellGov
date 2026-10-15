//! The staged-plan table a spawn loader hands the step loop.

use super::*;

/// A plan told apart from the others by its stack pointer.
fn plan(stack_pointer: u64) -> ChildInitPlan {
    ChildInitPlan {
        prx_modules: Vec::new(),
        tls: TlsArea::above(0).expect("a page at 0 is in range"),
        stack_pointer,
        run_hle_stubbed: false,
        ppu_tap: None,
    }
}

#[test]
fn tokens_name_plans_in_staging_order_and_each_is_taken_once() {
    let plans = ChildInitPlans::default();
    let a = plans.stage(plan(0xA));
    let b = plans.stage(plan(0xB));
    assert_eq!((a, b), (0, 1));

    let taken_b = plans.take(b).expect("staged");
    assert_eq!(taken_b.stack_pointer, 0xB);
    assert!(plans.take(b).is_none(), "a token is consumed on take");
    assert_eq!(plans.take(a).expect("staged").stack_pointer, 0xA);
}

#[test]
fn a_token_nothing_staged_yields_no_plan() {
    let plans = ChildInitPlans::default();
    assert!(plans.take(0).is_none());
    assert!(plans.take(u64::MAX).is_none());
}

#[test]
fn clones_share_one_staging_table() {
    let plans = ChildInitPlans::default();
    let loader_side = plans.clone();
    let token = loader_side.stage(plan(0xC));
    assert_eq!(
        plans
            .take(token)
            .expect("visible through the clone")
            .stack_pointer,
        0xC
    );
}
