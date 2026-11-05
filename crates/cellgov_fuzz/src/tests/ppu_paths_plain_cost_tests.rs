use super::*;

fn li(rt: u32, value: u16) -> u32 {
    (14 << 26) | (rt << 21) | u32::from(value)
}

/// A plain run over two `li`s, with the forwarded run in place of its
/// pricing run.
fn plain_and_priced() -> (PpuPathRun, PpuPathRun) {
    let words = [li(3, 1), li(4, 2)];
    let runs = run_all_paths(&words, &PpuState::new(), &[0; 64]).expect("the case runs");
    let plain = runs[0].clone();
    assert_eq!(plain.path, PpuExecutionPath::Plain);
    assert_eq!(plain.executed_pcs, [0, 4]);
    let priced = runs[1].clone();
    assert!(priced.consumed_cost.is_some_and(|cost| cost > 0));
    (plain, priced)
}

#[test]
fn a_pricing_run_that_matches_the_plain_run_gives_its_cost() {
    let (plain, priced) = plain_and_priced();
    assert_eq!(plain_cost(&plain, &priced), priced.consumed_cost);
}

#[test]
fn a_pricing_run_that_dispatched_other_pcs_leaves_the_plain_cost_unknown() {
    let (plain, mut priced) = plain_and_priced();
    priced.executed_pcs = vec![4, 0];
    assert_eq!(priced.stop, plain.stop);
    assert_eq!(priced.retired, plain.retired);
    assert_eq!(plain_cost(&plain, &priced), None);
}

#[test]
fn a_pricing_run_that_retired_another_count_leaves_the_plain_cost_unknown() {
    let (plain, mut priced) = plain_and_priced();
    priced.retired += 1;
    assert_eq!(plain_cost(&plain, &priced), None);
}

#[test]
fn a_pricing_run_that_stopped_elsewhere_leaves_the_plain_cost_unknown() {
    let (plain, mut priced) = plain_and_priced();
    priced.stop.pc = priced.stop.pc.map(|pc| pc + 4).or(Some(0));
    assert_ne!(priced.stop, plain.stop);
    assert_eq!(plain_cost(&plain, &priced), None);
}
