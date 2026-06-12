//! Rpcs3Decoder runner-string agreement with the Debug-derived form.

use super::*;
use strum::VariantArray;

/// Trip-wire: every variant's `as_runner_str()` must match
/// `format!("rpcs3-{:?}", v).to_lowercase()`.
#[test]
fn as_runner_str_matches_debug_derived_form() {
    for d in Rpcs3Decoder::VARIANTS {
        let debug_form = format!("rpcs3-{:?}", d).to_lowercase();
        assert_eq!(
            d.as_runner_str(),
            debug_form,
            "as_runner_str() drifted from Debug-derived form for {d:?}",
        );
    }
}
