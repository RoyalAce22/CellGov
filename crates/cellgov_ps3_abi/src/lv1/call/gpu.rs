//! The numbers of the GPU hypercalls.

/// `gpu_open`: one input and no output.
/// - r3 is the input, which lv1 keeps in its GPU object.
///
/// The lv1 handler [OFW-4.93 lv1 a:0x20E908] reads the input in this
/// form; see [`crate::lv1::gpu`].
pub const GPU_OPEN: u64 = 210;
