//! The runtime hands the hypervisor only a take whose SRR1 sets both
//! bits of `SRR1_MEDIATED_EXTERNAL` to test as a mediated take.

use cellgov_ps3_abi::lv1::call::SEND_EVENT_LOCALLY;
use cellgov_ps3_abi::lv1::interrupt_mask::MEDIATED_UNDELIVERED_PC;

use super::{call, run, runtime, setup, spawn, KERNEL_MSR, PLUG_BIT, PORT, SEND_SC};

/// The direct take sets SRR1 bit 33 and leaves bit 42 clear. Its SRR0 is
/// `MEDIATED_UNDELIVERED_PC`, at which the hypervisor refuses a mediated
/// take. So the run ends with no refusal: four calls, the take, and the
/// finish.
#[test]
fn a_direct_take_with_srr1_bit_33_alone_is_not_tested_as_a_mediated_take() {
    let mut rt = runtime(PLUG_BIT);
    let mut script = setup();
    script.push(call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT]));
    let (_, seen) = spawn(
        &mut rt,
        1,
        script,
        None,
        KERNEL_MSR,
        MEDIATED_UNDELIVERED_PC,
    );
    run(&mut rt);
    assert_eq!(rt.hypervisor().refusal(), None);
    let seen = seen.borrow();
    assert_eq!(seen.interrupts, 1);
    assert_eq!(seen.mediated_takes, 0, "EE = 1 at the send");
    assert!(!seen.external);
    assert_eq!(seen.writes.len(), 6);
}
