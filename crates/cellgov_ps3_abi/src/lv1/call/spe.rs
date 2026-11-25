//! The numbers of the logical SPE hypercalls, with the isolated SPE
//! calls of the SELF decryption path.

/// `construct_logical_spe`: seven inputs and six outputs.
///
/// The inputs:
/// - r3 to r7 are five page shifts.
/// - r8 is the vas id.
/// - r9 is the SPE type.
///
/// The outputs, from r4:
/// - the priv2 area's address;
/// - the problem state area's address;
/// - the local store's address;
/// - an unused value;
/// - the shadow page's address;
/// - the spe id.
///
/// Linux `arch/powerpc/platforms/ps3/spu.c`, `construct_spu`, makes the
/// call in this form. The lv1 handler [OFW-4.93 lv1 a:0x31AB6C] reads
/// the inputs and writes the outputs in this form; see
/// [`crate::lv1::spe`].
pub const CONSTRUCT_LOGICAL_SPE: u64 = 57;
