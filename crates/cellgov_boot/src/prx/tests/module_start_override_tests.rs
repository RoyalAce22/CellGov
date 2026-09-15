//! The `disable_module_start_hle_stubs` boot override at the one site
//! both the boot's pass and a child's pass reach.

use std::rc::Rc;

use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_event::UnitId;
use cellgov_mem::GuestMemory;
use cellgov_ppu::sprx::LoadedOpd;
use cellgov_time::Budget;

use super::{
    run_module_start, ModuleStartEnv, ModuleStartError, ModuleStartOutcome, PrxLoadInfo,
    HLE_STUBBED_MODULE_STARTS,
};

const OWNER: UnitId = UnitId::new(0);

fn env(run_hle_stubbed: bool) -> ModuleStartEnv {
    ModuleStartEnv {
        space: AddressSpaceId::BOOT,
        thread_owner: OWNER,
        pid: None,
        kctx_opd: 0,
        stack_pointer: 0x8000,
        break_pc: None,
        dump_mem_fault_ranges: Vec::new(),
        run_hle_stubbed,
        sink: Rc::new(crate::NullSink),
        ppu_tap: None,
    }
}

fn stubbed_module() -> PrxLoadInfo {
    PrxLoadInfo {
        name: HLE_STUBBED_MODULE_STARTS[0].to_string(),
        stem: "libsysutil".to_string(),
        base: 0x1_0000,
        data_end: 0x2_0000,
        toc: 0x1_8000,
        relocs_applied: 0,
        module_start: Some(LoadedOpd {
            code: 0x1_0000,
            toc: 0x1_8000,
        }),
        module_stop: None,
    }
}

fn runtime() -> Runtime {
    Runtime::new(GuestMemory::new(0x1_0000), Budget::new(1), 1)
}

#[test]
fn an_hle_stubbed_module_start_answers_without_running() {
    let outcome = run_module_start(&mut runtime(), &stubbed_module(), &env(false))
        .expect("the stub answers before the runtime is touched");
    assert_eq!(outcome, ModuleStartOutcome::HleStubbed);
}

#[test]
fn the_hle_stub_override_runs_the_stubbed_module_start() {
    // No PPU thread record exists for the owner, so the LLE path is
    // refused at its first act, the TLS thread-id seed.
    match run_module_start(&mut runtime(), &stubbed_module(), &env(true)) {
        Err(ModuleStartError::NoThreadRecord { owner }) => assert_eq!(owner, OWNER),
        other => panic!("the override did not reach the LLE path: {other:?}"),
    }
}
