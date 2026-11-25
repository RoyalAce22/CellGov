//! The runtime hands the hypervisor the decrementer that the taking unit
//! gives at its clock.

use cellgov_lv1::HypercallRefusal;
use cellgov_ps3_abi::lv1::call::SEND_EVENT_LOCALLY;

use super::{call, run_to_error, runtime, setup, spawn, EE_OFF_MSR, PLUG_BIT, PORT, SEND_SC};
use crate::runtime::types::StepError;

#[test]
fn a_mediated_take_on_a_unit_whose_decrementer_is_negative_stops_the_run() {
    let mut rt = runtime(PLUG_BIT);
    let mut script = setup();
    script.push(call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT]));
    let (unit, seen) = spawn(&mut rt, 1, script, None, EE_OFF_MSR, SEND_SC + 4);
    seen.borrow_mut().decrementer = 0x8000_0000;
    assert_eq!(run_to_error(&mut rt), StepError::HypercallRefused);
    assert_eq!(
        rt.hypervisor().refusal(),
        Some(HypercallRefusal::MediatedExternal {
            unit,
            pc: SEND_SC + 4,
            what: "a take whose decrementer is negative, which lv1's handler resets and can \
                   send to its trampoline slot 0x28B8",
        })
    );
    assert_eq!(seen.borrow().mediated_takes, 1);
}
