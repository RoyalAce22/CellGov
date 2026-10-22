use super::*;

/// A state whose registers, local store and PC differ from a new one's.
fn varied() -> SpuState {
    let mut state = SpuState::new();
    state.set_reg(3, [0x3c; 16]);
    state.ls[0x400] = 0x99;
    state.ls[SPU_LS_SIZE - 1] = 0x11;
    state.pc = 0x20;
    state
}

fn spare(pool: &LocalStores) -> usize {
    pool.spare.borrow().len()
}

#[test]
fn a_dropped_value_gives_its_local_store_back_and_the_next_state_reuses_it() {
    let pool = LocalStores::default();
    let held = pool.hold(pool.copy(&varied()));
    let pointer = held.ls.as_ptr();
    drop(held);
    assert_eq!(spare(&pool), 1);
    let fresh = pool.fresh();
    assert_eq!(spare(&pool), 0);
    assert_eq!(
        fresh.ls.as_ptr(),
        pointer,
        "fresh allocated past a spare buffer"
    );
}

#[test]
fn a_state_from_a_used_buffer_is_overwritten_in_full() {
    let pool = LocalStores::default();
    drop(pool.hold(pool.copy(&varied())));
    assert_eq!(pool.fresh(), SpuState::new());

    let mut other = SpuState::new();
    other.ls[0x400] = 0x01;
    other.pc = 0x80;
    drop(pool.hold(pool.copy(&varied())));
    assert_eq!(pool.copy(&other), other);
}

#[test]
fn a_snapshot_gives_its_local_store_back_and_its_shell_gives_nothing() {
    let pool = LocalStores::default();
    let mut state = pool.hold(pool.copy(&varied()));
    let snapshot = pool.hold(SpuObservableSnapshot::capture_taking(&mut state));
    drop(state);
    assert_eq!(spare(&pool), 0, "the emptied state gave a buffer back");
    drop(snapshot);
    assert_eq!(spare(&pool), 1);
}

#[test]
fn a_buffer_too_small_for_a_local_store_is_dropped() {
    let pool = LocalStores::default();
    pool.give(vec![0; SPU_LS_SIZE - 1]);
    assert_eq!(spare(&pool), 0);
    pool.give(Vec::with_capacity(SPU_LS_SIZE));
    assert_eq!(spare(&pool), 1);
}
