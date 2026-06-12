//! Decoder runner-string values carried into observation metadata.

use super::*;

#[test]
fn decoder_format_in_metadata() {
    assert_eq!(
        Rpcs3Decoder::Interpreter.as_runner_str(),
        "rpcs3-interpreter"
    );
    assert_eq!(Rpcs3Decoder::Llvm.as_runner_str(), "rpcs3-llvm");
}
