//! A raise that wakes a paused thread whose `pause` saved EE = 0 gives
//! the woken thread the mediated external exception.

use std::rc::Rc;

use cellgov_ps3_abi::lv1::call::{PAUSE, SEND_EVENT_LOCALLY};

use super::{
    call, register, run, runtime, setup, spawn, Step, EE_OFF_MSR, PLUG_BIT, PORT, SEND_SC,
};

/// lv1 wakes thread 1 as for a pause that saved EE = 1. Here the saved
/// EE = 0, so the return sets LPCR\[MER\] and changes no saved register
/// [OFW-4.93 lv1 a:0x191C-0x1928]. r11 and r12 keep the saved SRR0 and
/// SRR1 [OFW-4.93 lv1 a:0x1038-0x103C]. The steps are four calls, the
/// mediated take, and the finish.
#[test]
fn a_raise_that_wakes_a_paused_thread_with_ee_off_makes_it_take_the_mediated_interrupt() {
    let mut rt = runtime(PLUG_BIT);
    let mut script = setup();
    script.push(call(SEND_SC, PAUSE, &[0]));
    let (_, thread_1) = spawn(&mut rt, 1, script, None, EE_OFF_MSR, SEND_SC + 4);
    let send = vec![Step::AwaitPause, call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT])];
    let thread_0 = register(&mut rt, 0, send, Some(Rc::clone(&thread_1)));
    run(&mut rt);
    assert_eq!(thread_0.borrow().interrupts, 0);
    assert_eq!(thread_0.borrow().mediated_takes, 0);
    let thread_1 = thread_1.borrow();
    assert_eq!(thread_1.mediated_takes, 1);
    assert_eq!(thread_1.interrupts, 0, "EE = 0 at the pause");
    assert!(!thread_1.mediated);
    assert!(!thread_1.external);
    assert_eq!(thread_1.writes.len(), 6);
    let after_pause = &thread_1.writes[4];
    assert_eq!(
        &after_pause[after_pause.len() - 2..],
        &[(11, 0), (12, EE_OFF_MSR)]
    );
}
