//! The long PPU sweep, run by the scheduled fuzz workflow.

use cellgov_fuzz::ppu_sweep::PPU_SWEEP;

use super::spu_sweep::hold_to_the_smoke_floors;

#[test]
#[ignore = "runs the long PPU sweep; the scheduled fuzz workflow runs it"]
fn the_ppu_sweep_reaches_the_smoke_floors_and_every_finding_is_promoted() {
    hold_to_the_smoke_floors(&PPU_SWEEP);
}
