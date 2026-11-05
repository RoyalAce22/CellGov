//! A shared-memory window that `sys_mmapper_map_shared_memory` (334)
//! installs or `sys_mmapper_unmap_shared_memory` (335) removes drops
//! every unit's predecoded code over the window.

use std::cell::RefCell;
use std::rc::Rc;

use cellgov_ps3_abi::lv2::syscall::{
    MMAPPER_ALLOCATE_SHARED_MEMORY, MMAPPER_MAP_SHARED_MEMORY, MMAPPER_UNMAP_SHARED_MEMORY,
};

use super::lv2_apply::reserve_mmapper_areas;
use super::*;

const MEM_ID_PTR: u64 = 0x4000;
const UNMAP_ID_PTR: u64 = 0x4010;
const MAP_ADDR: u64 = 0x5000_0000;
const SIZE: u64 = 0x1_0000;

/// The `(addr, len)` of every `invalidate_code` call a recorder saw.
type Calls = Rc<RefCell<Vec<(u64, u64)>>>;

/// Never runs; records every `invalidate_code` call it receives.
#[derive(Clone)]
struct InvalidationRecorder {
    id: UnitId,
    calls: Calls,
}

impl ExecutionUnit for InvalidationRecorder {
    type Snapshot = ();

    fn unit_id(&self) -> UnitId {
        self.id
    }

    fn status(&self) -> UnitStatus {
        UnitStatus::Finished
    }

    fn run_until_yield(
        &mut self,
        budget: Budget,
        _ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(budget.raw()),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }

    fn invalidate_code(&mut self, addr: u64, len: u64) {
        self.calls.borrow_mut().push((addr, len));
    }

    fn caches_code(&self) -> bool {
        true
    }

    fn snapshot(&self) {}
}

fn call(rt: &mut Runtime, unit: UnitId, number: u64, args: [u64; 4]) -> Option<u64> {
    let [a, b, c, d] = args;
    rt.dispatch_lv2_request(
        cellgov_lv2::Lv2Request::Unsupported {
            number,
            args: [a, b, c, d, 0, 0, 0, 0],
        },
        unit,
    );
    rt.registry_mut().drain_syscall_return(unit)
}

/// Counts the drops the recorder saw over exactly the window's range.
fn window_drops(calls: &Calls) -> usize {
    calls
        .borrow()
        .iter()
        .filter(|&&call| call == (MAP_ADDR, SIZE))
        .count()
}

#[test]
fn mapping_and_unmapping_a_window_each_drop_its_predecoded_code() {
    let mut rt = build(0x10000, 1, 100);
    let source = rt
        .registry_mut()
        .register_with(|id| CountingUnit::new(id, 1));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let recorder_calls = Rc::clone(&calls);
    rt.registry_mut()
        .register_with(move |id| InvalidationRecorder {
            id,
            calls: recorder_calls,
        });
    reserve_mmapper_areas(&mut rt, source, 0x4100);
    let keyless = cellgov_ps3_abi::lv2::memory::SYS_MMAPPER_NO_SHM_KEY;
    let flag_64k = cellgov_ps3_abi::lv2::memory::page_size::FLAG_64K;
    assert_eq!(
        call(
            &mut rt,
            source,
            MMAPPER_ALLOCATE_SHARED_MEMORY,
            [keyless, SIZE, flag_64k, MEM_ID_PTR]
        ),
        Some(0)
    );
    let mem_id = rt
        .memory()
        .read(super::lv2_apply::range4(MEM_ID_PTR))
        .map(|b| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
        .expect("the out-parameter is mapped");
    assert_eq!(window_drops(&calls), 0, "no window exists yet");

    let map = [MAP_ADDR, u64::from(mem_id), 0, 0];
    assert_eq!(
        call(&mut rt, source, MMAPPER_MAP_SHARED_MEMORY, map),
        Some(0)
    );
    assert_eq!(window_drops(&calls), 1, "the install drops the window");

    let unmap = [MAP_ADDR, UNMAP_ID_PTR, 0, 0];
    assert_eq!(
        call(&mut rt, source, MMAPPER_UNMAP_SHARED_MEMORY, unmap),
        Some(0)
    );
    assert_eq!(window_drops(&calls), 2, "the removal drops the window");
}
