//! The PPU state generator draws the control state of the FPSCR and the
//! VSCR. The cases of one smoke campaign start under:
//! - every rounding mode;
//! - every exception enable;
//! - each NJ and SAT value.

use super::*;

use cellgov_ps3_abi::hw::ppu::{VSCR_NJ, VSCR_SAT};

/// The case count of the smallest PPU smoke campaign.
const SMOKE_CASES: u64 = 300;

fn drawn_states() -> Vec<PpuState> {
    (0..SMOKE_CASES)
        .map(|case| {
            let mut rng = Rng::for_case(crate::CAMPAIGN_VERSION, 1, case);
            random_state(&mut rng).expect("a state")
        })
        .collect()
}

#[test]
fn the_drawn_states_reach_every_rounding_mode_enable_and_vector_mode() {
    let states = drawn_states();
    assert_eq!(states.len() as u64, SMOKE_CASES);
    let rounding = states
        .iter()
        .map(|state| state.fpscr() & PPC_FPSCR_RN)
        .collect::<BTreeSet<_>>();
    assert_eq!(rounding, BTreeSet::from([0, 1, 2, 3]));
    let enables = states.iter().fold(0, |seen, state| {
        seen | (state.fpscr() & PPC_FPSCR_ENABLE_BITS)
    });
    assert_eq!(enables, PPC_FPSCR_ENABLE_BITS);
    assert!(states.iter().any(|state| state.fpscr() == 0));
    let vscr = states.iter().map(PpuState::vscr).collect::<BTreeSet<_>>();
    assert_eq!(
        vscr,
        BTreeSet::from([0, VSCR_SAT, VSCR_NJ, VSCR_NJ | VSCR_SAT])
    );
}

#[test]
fn a_drawn_fpscr_holds_control_bits_only() {
    for state in drawn_states() {
        assert_eq!(
            state.fpscr() & !(PPC_FPSCR_RN | PPC_FPSCR_ENABLE_BITS),
            0,
            "{:#010x}",
            state.fpscr()
        );
    }
}
