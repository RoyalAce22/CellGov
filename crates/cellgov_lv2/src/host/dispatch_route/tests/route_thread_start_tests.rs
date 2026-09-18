//! `sys_ppu_thread_start` (53) through `Lv2Host::dispatch`.

use super::*;

#[test]
fn ppu_thread_start_on_an_unknown_thread_id_is_esrch() {
    let mut host = Lv2Host::new();
    let rt = FakeRuntime::new(0x10000);
    let result = host.dispatch(
        Lv2Request::PpuThreadStart { target: 0x9999 },
        UnitId::new(0),
        &rt,
    );
    assert_eq!(result, Lv2Dispatch::immediate(errno::CELL_ESRCH.into()));
}
