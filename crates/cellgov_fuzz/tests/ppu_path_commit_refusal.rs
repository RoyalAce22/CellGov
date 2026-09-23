//! Tests typed PPU commit refusals across all execution paths.

use cellgov_effects::Effect;
use cellgov_fuzz::ppu_paths::{run_one_path, PpuExecutionPath, PpuPathError};
use cellgov_ppu::observation::PpuObservationError;
use cellgov_ppu::state::PpuState;

fn stw(rs: u32, ra: u32, offset: u16) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(offset)
}

#[test]
fn every_path_refuses_a_mapped_write_outside_the_observed_data() {
    let mut state = PpuState::new();
    state.set_gpr(3, 0x1122_3344);
    state.set_gpr(4, 4);

    for path in [
        PpuExecutionPath::Plain,
        PpuExecutionPath::Forwarded,
        PpuExecutionPath::Quickened,
        PpuExecutionPath::Fused,
    ] {
        let error = run_one_path(path, &[stw(3, 4, 0)], &state, &[0; 64])
            .expect_err("a code-region write must not be reported as a successful observation");

        match error {
            PpuPathError::CommitRefusal {
                path: refused,
                error:
                    PpuObservationError::WriteOutOfRange {
                        addr: 4, len: 4, ..
                    },
                staged_effects,
                committed_effects,
            } => {
                assert_eq!(refused, path);
                assert!(staged_effects
                    .iter()
                    .any(|effect| matches!(effect, Effect::SharedWriteIntent { .. })));
                assert!(committed_effects.is_empty());
            }
            other => panic!("wrong refusal for {path:?}: {other:?}"),
        }
    }
}
