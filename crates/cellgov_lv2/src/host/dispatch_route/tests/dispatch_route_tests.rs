//! Raw-numbered syscall routing through `Lv2Host::dispatch`: TTY writes, time queries, mmapper memory arms, PRX module bookkeeping, and stub/unsupported fallbacks.

use cellgov_effects::Effect;

use cellgov_event::UnitId;

use cellgov_mem::ByteRange;

use cellgov_ps3_abi::lv2::errno;

use crate::dispatch::Lv2Dispatch;

use crate::host::test_support::FakeRuntime;

use crate::host::Lv2Host;

use crate::request::Lv2Request;

#[path = "route_io_time_tests.rs"]
mod io_time;
#[path = "route_memory_tests.rs"]
mod memory;
#[path = "route_mmapper_ext_tests.rs"]
mod mmapper_ext;
#[path = "route_process_spu_tests.rs"]
mod process_spu;
#[path = "route_prx_tests.rs"]
mod prx;
#[path = "route_stub_tests.rs"]
mod stub;
