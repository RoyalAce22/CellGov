use super::*;
use crate::host::test_support::seed_primary_ppu;

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "expire_wait.untimed_reason")]
fn a_uart_timer_entry_is_refused_as_an_untimed_reason() {
    let mut host = Lv2Host::new();
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    host.expire_wait(Lv2BlockReason::Uart, src, GuestTicks::ZERO);
}

#[cfg(not(debug_assertions))]
#[test]
fn a_uart_timer_entry_is_refused_without_a_wake_in_release() {
    let mut host = Lv2Host::new();
    let src = UnitId::new(0);
    seed_primary_ppu(&mut host, src);
    let out = host.expire_wait(Lv2BlockReason::Uart, src, GuestTicks::ZERO);
    assert!(
        out.woken_unit_ids.is_empty(),
        "no ETIMEDOUT for an untimed park"
    );
    assert!(out.response_updates.is_empty());
    assert!(out.effects.is_empty());
    assert_eq!(
        host.observability()
            .invariant_break_sites
            .get("expire_wait.untimed_reason"),
        Some(&1)
    );
    assert!(
        host.observability().wait_timeout_expiries.is_empty(),
        "a refused expiry is not a timeout"
    );
}
