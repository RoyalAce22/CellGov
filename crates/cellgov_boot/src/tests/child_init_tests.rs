//! The staged-plan table a spawn loader hands the step loop.

use super::*;

fn plan(kctx: u64) -> ChildInitPlan {
    ChildInitPlan {
        prx_modules: Vec::new(),
        kctx_opd: kctx,
        stack_pointer: 0x1000,
        run_hle_stubbed: false,
    }
}

#[test]
fn tokens_name_plans_in_staging_order_and_each_is_taken_once() {
    let plans = ChildInitPlans::default();
    let a = plans.stage(plan(0xA));
    let b = plans.stage(plan(0xB));
    assert_eq!((a, b), (0, 1));

    let taken_b = plans.take(b).expect("staged");
    assert_eq!(taken_b.kctx_opd, 0xB);
    assert!(plans.take(b).is_none(), "a token is consumed on take");
    assert_eq!(plans.take(a).expect("staged").kctx_opd, 0xA);
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
            .kctx_opd,
        0xC
    );
}
