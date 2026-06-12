//! Multi-primitive determinism canaries over RSX and atomic content.

use super::*;

#[test]
fn multi_primitive_determinism_canary_with_rsx_content() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    fn run_once() -> (u64, u64) {
        let mut rt = build(256, 4, 2000);
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x11),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::LoadImm(0x22),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x33),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x77),
                    FakeOp::SharedStore { addr: 0x80, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| RsxFlipSpinnerUnit {
            id,
            steps: Cell::new(0),
            count: 10,
        });

        for _ in 0..500 {
            match rt.step() {
                Ok(step) => {
                    let _ = rt.commit_step(&step.result, &step.effects);
                }
                Err(_) => break,
            }
        }
        (rt.memory().content_hash(), rt.sync_state_hash())
    }

    let run_a = run_once();
    let run_b = run_once();
    assert_eq!(
        run_a, run_b,
        "extended multi-primitive canary must produce byte-identical final (memory, sync) hashes across runs"
    );
}

#[test]
fn multi_primitive_determinism_canary_rsx_per_step_hash_sequence_stable() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    fn run_once() -> Vec<u64> {
        let mut rt = build(256, 4, 2000);
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x11),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| RsxFlipSpinnerUnit {
            id,
            steps: Cell::new(0),
            count: 5,
        });
        let mut hashes = vec![rt.sync_state_hash()];
        for _ in 0..500 {
            match rt.step() {
                Ok(step) => {
                    let _ = rt.commit_step(&step.result, &step.effects);
                    hashes.push(rt.sync_state_hash());
                }
                Err(_) => break,
            }
        }
        hashes
    }

    let run_a = run_once();
    let run_b = run_once();
    assert_eq!(
        run_a, run_b,
        "per-step sync_state_hash sequence must be byte-identical across two runs"
    );
    assert!(
        run_a.len() >= 8,
        "canary must have at least a handful of commits for a meaningful per-step comparison, got {}",
        run_a.len()
    );
}

#[test]
fn multi_primitive_determinism_canary_with_atomic_content() {
    use cellgov_exec::fake_isa::{FakeIsaUnit, FakeOp};
    fn run_once() -> (u64, u64) {
        let mut rt = build(256, 4, 500);
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x11),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::LoadImm(0x22),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x33),
                    FakeOp::ReservationAcquire { line_addr: 0 },
                    FakeOp::ConditionalStore { addr: 0, len: 4 },
                    FakeOp::End,
                ],
            )
        });
        rt.registry_mut().register_with(|id| {
            FakeIsaUnit::new(
                id,
                vec![
                    FakeOp::LoadImm(0x77),
                    FakeOp::SharedStore { addr: 0x80, len: 4 },
                    FakeOp::End,
                ],
            )
        });

        for _ in 0..200 {
            match rt.step() {
                Ok(step) => {
                    let _ = rt.commit_step(&step.result, &step.effects);
                }
                Err(_) => break,
            }
        }
        (rt.memory().content_hash(), rt.sync_state_hash())
    }

    let run_a = run_once();
    let run_b = run_once();
    assert_eq!(
        run_a, run_b,
        "multi-primitive atomic canary must produce byte-identical final (memory, sync) hashes across runs"
    );
}
