//! A mediated take that the hypervisor answers with no vector entry: the
//! guest narrowed its mask after the return that set LPCR\[MER\], so lv1's
//! handler finds no bit and returns the thread to the interrupted
//! instruction.

use cellgov_mem::{ByteRange, GuestAddr};
use cellgov_ps3_abi::lv1::call::SEND_EVENT_LOCALLY;
use cellgov_ps3_abi::lv1::interrupt::IRQ_MASK_OFFSET;

use super::{
    call, run, runtime, setup, spawn, Shared, BITMAP, EE_OFF_MSR, PLUG_BIT, PORT, SEND_SC,
};
use crate::runtime::Runtime;

/// The steps up to the end of the send: three setup calls and the send.
const STEPS_TO_SEND: usize = 4;

/// A runtime whose thread 1 sends on its own plug with EE = 0, so the
/// send's return sets LPCR\[MER\]. The run stops after the send, and the
/// mask then holds `mask_at_take`, as a guest store leaves it.
fn sent_then_masked(mask_at_take: u64) -> (Runtime, Shared) {
    let mut rt = runtime(PLUG_BIT);
    let mut script = setup();
    script.push(call(SEND_SC, SEND_EVENT_LOCALLY, &[PORT]));
    let (_, seen) = spawn(&mut rt, 1, script, None, EE_OFF_MSR, SEND_SC + 4);
    for _ in 0..STEPS_TO_SEND {
        let mut step = rt.step().expect("a step before the take");
        rt.commit_step(&mut step).expect("commit");
    }
    assert!(seen.borrow().mediated, "the send's return set LPCR[MER]");
    let range = ByteRange::new(GuestAddr::new(BITMAP + IRQ_MASK_OFFSET), 8).expect("a range");
    rt.memory_mut()
        .apply_commit(range, &mask_at_take.to_be_bytes())
        .expect("the mask store");
    (rt, seen)
}

/// lv1's handler skips the vector entry when the shadow and the mask
/// share no bit [OFW-4.93 lv1 a:0x1A84-0x1AD4].
#[test]
fn a_mediated_take_after_the_guest_cleared_its_mask_resumes_the_thread() {
    let (mut rt, seen) = sent_then_masked(0);
    run(&mut rt);
    assert_eq!(rt.hypervisor().refusal(), None);
    let seen = seen.borrow();
    assert_eq!(seen.mediated_takes, 1);
    assert_eq!((seen.resumes, seen.vector_entries), (1, 0));
    assert!(!seen.mediated, "the handler's LPCR reload clears MER");
}

#[test]
fn a_mediated_take_whose_mask_kept_its_bit_enters_the_vector() {
    let (mut rt, seen) = sent_then_masked(PLUG_BIT);
    run(&mut rt);
    assert_eq!(rt.hypervisor().refusal(), None);
    let seen = seen.borrow();
    assert_eq!(seen.mediated_takes, 1);
    assert_eq!((seen.resumes, seen.vector_entries), (0, 1));
}
