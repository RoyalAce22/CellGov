//! The decrementer wake of a thread paused in mode 0: the order of the
//! wake against a plug raise, the DEC = 0 write of the wake, and the
//! deadline that a plug wake cancels.

use std::rc::Rc;

use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv1::call::{PAUSE, SEND_EVENT_LOCALLY};
use cellgov_ps3_abi::lv1::interrupt::IRQ_MASK_OFFSET;

use super::{
    call, register, run, run_to_error, runtime, setup, spawn, HypercallRefusal, Step, StepError,
    BITMAP, KERNEL_MSR, PLUG_BIT, PORT, SEND_SC, STEP_CAP,
};

/// lv1's return writes DEC = 0 after a timer wake
/// [OFW-4.93 lv1 a:0x2C425C-0x2C4274], whose timer sets the byte that the
/// return tests [OFW-4.93 lv1 a:0x2C5D2C]. The only thread pauses at tick
/// 4, so the run moves guest time to the deadline at tick 50.
#[test]
fn a_decrementer_wake_writes_dec_0_at_its_deadline() {
    let mut rt = runtime(PLUG_BIT);
    let mut script = setup();
    script.push(call(SEND_SC, PAUSE, &[0]));
    let thread_1 = register(&mut rt, 1, script, None);
    thread_1.borrow_mut().decrementer_deadlines = vec![50];
    run(&mut rt);
    let thread_1 = thread_1.borrow();
    assert_eq!(thread_1.decrementer_writes, vec![(0, 50)]);
    assert_eq!(thread_1.interrupts, 0);
}

/// Thread 1 pauses at tick 4 with its deadline at tick 8. Thread 0's send
/// runs from tick 4 to tick 14. lv1's timer ends the pause at tick 8
/// [OFW-4.93 lv1 a:0x2C5CC8-0x2C5D3C], so the send at tick 14 raises the
/// plug of a thread that is no longer paused. That thread has not run
/// from tick 8, so lv1's interprocessor interrupt to it is a refusal.
#[test]
fn a_decrementer_deadline_inside_a_raising_step_wakes_the_thread_before_the_raise() {
    let mut rt = runtime(PLUG_BIT);
    let mut script = setup();
    script.push(call(SEND_SC, PAUSE, &[0]));
    let thread_1 = register(&mut rt, 1, script, None);
    thread_1.borrow_mut().decrementer_deadlines = vec![8];
    let mut send_args = [0; 9];
    send_args[0] = SEND_EVENT_LOCALLY;
    send_args[1] = PORT;
    let send = vec![Step::AwaitPause, Step::CallCosting(10, SEND_SC, send_args)];
    let (sender, _) = spawn(&mut rt, 0, send, Some(Rc::clone(&thread_1)), KERNEL_MSR, 0);
    assert_eq!(run_to_error(&mut rt), StepError::HypercallRefused);
    assert!(
        matches!(
            rt.hypervisor().refusal(),
            Some(HypercallRefusal::UnmodelledInput {
                number: SEND_EVENT_LOCALLY,
                unit,
                pc: SEND_SC,
                what: "an interprocessor interrupt to a thread whose unit gives no running registers",
            }) if unit == sender
        ),
        "{:?}",
        rt.hypervisor().refusal()
    );
    let thread_1 = thread_1.borrow();
    assert_eq!(thread_1.decrementer_writes, vec![(0, 8)]);
    assert_eq!(thread_1.interrupts, 0);
}

#[test]
fn a_pause_after_a_plug_wake_keeps_only_its_own_decrementer_deadline() {
    let mut rt = runtime(PLUG_BIT);
    let mut script = setup();
    script.push(call(SEND_SC, PAUSE, &[0]));
    script.push(call(SEND_SC, PAUSE, &[0]));
    let thread_1 = register(&mut rt, 1, script, None);
    thread_1.borrow_mut().decrementer_deadlines = vec![100, 200];
    let send = vec![Step::AwaitPause, call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT])];
    register(&mut rt, 0, send, Some(Rc::clone(&thread_1)));
    for _ in 0..STEP_CAP {
        if thread_1.borrow().interrupts == 1 {
            break;
        }
        let mut step = rt.step().expect("a step before the plug wake's take");
        rt.commit_step(&mut step).expect("commit");
    }
    assert_eq!(thread_1.borrow().interrupts, 1);
    let mask = ByteRange::new(GuestAddr::new(BITMAP + IRQ_MASK_OFFSET), 8).expect("a range");
    rt.memory_mut()
        .apply_commit(mask, &0u64.to_be_bytes())
        .expect("the mask");
    run(&mut rt);
    let thread_1 = thread_1.borrow();
    assert_eq!(thread_1.pauses, 2);
    assert_eq!(thread_1.interrupts, 1);
    assert_eq!(thread_1.decrementer_writes, vec![(0, 200)]);
    assert_eq!(rt.timer_wakes.displacement_count(), 0);
    assert_eq!(rt.lv2_host().observability().invariant_break_count, 0);
}
