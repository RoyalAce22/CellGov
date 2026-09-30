use super::*;

/// A state whose every component differs from [`SpuState::new`].
fn varied() -> SpuState {
    let mut state = SpuState::new();
    for k in 0..SPU_REG_COUNT {
        state.set_reg(k, [k as u8; 16]);
    }
    state.ls[0x100] = 0x5a;
    state.ls[SPU_LS_SIZE - 1] = 0xa5;
    state.pc = 0x40;
    state.channels.in_mbox = vec![7, 9];
    state.channels.mfc_lsa = 0x80;
    state.set_reservation(Some(cellgov_sync::ReservedLine::containing(0x1000)));
    state.set_fpscr(cellgov_ps3_abi::hw::spu_fpscr::FPSCR_DEFINED);
    state.set_lslr(0x3_fff0);
    state.signals[0].pending = true;
    state.signals[1].word = 0x1234;
    state.stop = Some(SpuStop::new(
        crate::stop::SpuStopKind::Stop,
        0x10,
        0x80,
        0x3_fff0,
    ));
    state.set_interrupts_enabled(true);
    state.set_srr0(0x44);
    state
}

#[test]
fn clone_from_equals_clone_and_keeps_the_local_store_buffer() {
    let source = varied();
    let mut target = SpuState::new();
    let buffer = target.ls.as_ptr();
    target.clone_from(&source);
    assert_eq!(target, source.clone());
    assert_eq!(
        target.ls.as_ptr(),
        buffer,
        "clone_from reallocated the local store"
    );
}

#[test]
fn an_owned_capture_equals_a_borrowed_one() {
    let state = varied();
    assert_eq!(
        SpuObservableSnapshot::capture_owned(state.clone()),
        SpuObservableSnapshot::capture(&state)
    );
}

#[test]
fn a_snapshot_matches_exactly_the_state_it_equals_as_a_capture() {
    let state = varied();
    let snapshot = SpuObservableSnapshot::capture(&state);
    assert!(snapshot.matches(&state));

    let mut local_store = state.clone();
    local_store.ls[0x200] ^= 1;
    assert!(!snapshot.matches(&local_store));

    let mut counter = state.clone();
    counter.pc += 4;
    assert!(!snapshot.matches(&counter));

    let mut channel = state.clone();
    channel.channels.in_mbox.pop();
    assert!(!snapshot.matches(&channel));

    // The two components an observation comparison ignores still count.
    let mut limit = state.clone();
    limit.set_lslr(SPU_LSLR_FULL);
    assert!(!snapshot.matches(&limit));

    let mut stopped = state;
    stopped.stop = None;
    assert!(!snapshot.matches(&stopped));
}
