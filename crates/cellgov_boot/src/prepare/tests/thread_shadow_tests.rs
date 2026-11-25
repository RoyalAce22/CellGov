//! The predecoded code of each PPU thread the factory builds.

use cellgov_event::UnitId;
use cellgov_lv2::PpuThreadInitState;
use cellgov_ppu::shadow::PredecodedShadow;
use cellgov_ppu::PpuExecutionUnit;

use super::{ppu_thread_unit, ThreadShadow};
use crate::prepare::types::BootDebugOptions;

/// `li r3,1 ; blr`.
const TEXT: [u8; 8] = [0x38, 0x60, 0x00, 0x01, 0x4E, 0x80, 0x00, 0x20];

pub(super) fn created_thread(thread_shadow: &ThreadShadow) -> PpuExecutionUnit {
    let init = PpuThreadInitState {
        entry_code: 0,
        entry_toc: 0,
        arg: 0,
        extra_args: [0; 7],
        stack_top: 0x1000,
        tls_base: 0,
        lr_sentinel: 0,
    };
    let debug_opts = BootDebugOptions {
        dump_at_pc: None,
        dump_skip: 0,
        profile_pairs: false,
        ppu_tap: None,
    };
    ppu_thread_unit(UnitId::new(1), &init, &debug_opts, thread_shadow)
}

#[test]
fn a_created_thread_shares_the_offered_shadow() {
    let shadow = PredecodedShadow::build(0, &TEXT);
    let thread_shadow = ThreadShadow::default();
    thread_shadow.offer(Some(&shadow));
    let unit = created_thread(&thread_shadow);
    assert!(
        unit.instruction_shadow()
            .is_some_and(|held| held.shares_decode(&shadow)),
        "the created thread must fetch through the offered shadow's decode",
    );
}

#[test]
fn a_withdraw_reaches_only_the_threads_built_after_it() {
    let shadow = PredecodedShadow::build(0, &TEXT);
    let thread_shadow = ThreadShadow::default();
    thread_shadow.offer(Some(&shadow));
    let before = created_thread(&thread_shadow);
    thread_shadow.withdraw();
    assert!(created_thread(&thread_shadow)
        .instruction_shadow()
        .is_none());
    assert!(
        before
            .instruction_shadow()
            .is_some_and(|held| held.shares_decode(&shadow)),
        "a thread built before the withdraw keeps the offered shadow",
    );
}

#[test]
fn a_withdraw_before_the_offer_keeps_the_shadow_withdrawn() {
    let shadow = PredecodedShadow::build(0, &TEXT);
    let thread_shadow = ThreadShadow::default();
    thread_shadow.withdraw();
    thread_shadow.offer(Some(&shadow));
    assert!(created_thread(&thread_shadow)
        .instruction_shadow()
        .is_none());
}
