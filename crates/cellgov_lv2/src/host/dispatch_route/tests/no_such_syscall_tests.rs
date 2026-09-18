//! Dispatch behavior for an `r11` value outside the LV2 syscall table.

use super::*;
use cellgov_ps3_abi::lv2::syscall::SYSCALL_TABLE_SLOTS;

#[test]
fn out_of_table_dispatch_is_not_counted_as_an_unmodeled_slot() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(256);
    let req = Lv2Request::NoSuchSyscall {
        number: SYSCALL_TABLE_SLOTS,
        args: [0; 8],
    };
    let result = host.dispatch(req, UnitId::new(0), &rt);
    assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_ENOSYS.into()));
    assert!(host.observability().unsupported_syscalls.is_empty());
    assert_eq!(
        host.invariant_break_site_count("dispatch.no_such_syscall"),
        1
    );
    assert_eq!(
        host.invariant_break_site_count("dispatch.unsupported_stub"),
        0
    );
}
