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

/// `get_spe_irq_outlet`: two inputs and one output.
///
/// The inputs are the spe id in r3 and the interrupt class in r4. The
/// output in r4 is the id of the class's irq outlet.
///
/// Linux `arch/powerpc/platforms/ps3/interrupt.c`, `ps3_spe_irq_setup`,
/// makes the call in this form. The lv1 handler
/// [OFW-4.93 lv1 a:0x2FC968] reads the inputs and writes the output in
/// this form; see [`crate::lv1::spe`].
pub const GET_SPE_IRQ_OUTLET: u64 = 78;

/// `set_spe_transition_notifier`: three inputs and no output.
///
/// The inputs:
/// - r3 is the spe id.
/// - r4 is the mask.
/// - r5 is the id of the event receive port, or 0 for no port.
///
/// Linux `arch/powerpc/include/asm/lv1call.h` gives the call three inputs
/// and no output. The lv1 handler [OFW-4.93 lv1 a:0x2FCF68] reads the
/// inputs in this form; see [`crate::lv1::spe`].
pub const SET_SPE_TRANSITION_NOTIFIER: u64 = 64;

/// `logical_spu_set_run_timeout`: two inputs and no output.
///
/// The inputs are the spe id in r3 and the run time limit in time base
/// ticks in r4.
///
/// The OtherOS++ copy of `lv1call.h` gives the call two inputs and no
/// output, as `undocumented_function_138`. The lv1 handler
/// [OFW-4.93 lv1 a:0x31BD54] reads the inputs in this form; see
/// [`crate::lv1::spe`].
pub const LOGICAL_SPU_SET_RUN_TIMEOUT: u64 = 138;

/// `set_spe_interrupt_mask`: three inputs and no output.
///
/// The inputs:
/// - r3 is the spe id.
/// - r4 is the interrupt class.
/// - r5 is the interrupt mask of that class.
///
/// Linux `arch/powerpc/platforms/ps3/spu.c`, `int_mask_set`, makes the
/// call in this form. The lv1 handler [OFW-4.93 lv1 a:0x2FE570] reads the
/// inputs in this form; see [`crate::lv1::spe`].
pub const SET_SPE_INTERRUPT_MASK: u64 = 61;
