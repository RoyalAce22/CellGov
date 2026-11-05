//! The double-format facts `cellgov_float` keeps its own copy of, held
//! equal to the `cellgov_ps3_abi` constants the PPU arms read.

use cellgov_float::{ppc_nan_result, PPC_GENERATED_NAN};
use cellgov_ps3_abi::hw::ppc_isa::{PPC_F64_QUIET_BIT, PPC_GENERATED_QNAN_F64};

#[test]
fn the_engine_and_the_abi_agree_on_the_generated_qnan_and_the_quiet_bit() {
    assert_eq!(PPC_GENERATED_NAN, PPC_GENERATED_QNAN_F64);
    let snan = 0xFFF0_0000_0000_0001;
    assert_eq!(
        ppc_nan_result(Some(snan), None, None),
        Some(snan | PPC_F64_QUIET_BIT)
    );
}
