//! A module_start unit counts its retirements when the boot profiles, so
//! the operand profile of a run holds the code module_start ran.

use std::rc::Rc;

use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_event::UnitId;
use cellgov_lv2::PpuThreadAttrs;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::sprx::LoadedOpd;
use cellgov_ppu::PpuExecutionUnit;
use cellgov_time::Budget;

use super::{run_module_start, ModuleStartEnv, ModuleStartOutcome, PrxLoadInfo};
use crate::prx::TlsArea;

/// The thread the module_start unit runs as. No unit holds this id.
const OWNER: UnitId = UnitId::new(7);
/// The module_start entry point.
const ENTRY: u64 = 0x1_0000;
/// The base of the TLS area, clear of the entry.
const TLS: u64 = 0x2_0000;
/// `li r3, 0`, then `blr`.
const CODE: [u32; 2] = [0x3860_0000, 0x4E80_0020];

/// Runs the two-word module_start and returns the retirements the
/// operand profiles of the run's PPU units counted.
fn profiled_retirements(profile_mode: bool) -> u64 {
    let mut mem = GuestMemory::new(0x4_0000);
    for (i, word) in CODE.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(ENTRY + 4 * i as u64), 4).unwrap();
        mem.apply_commit(range, &word.to_be_bytes()).unwrap();
    }
    mem.declare_code(ENTRY, 8);
    mem.unmap_below(ENTRY);
    mem.restrict_fetch();
    let mut rt = Runtime::new(mem, Budget::new(1), 1_000);
    rt.lv2_host_mut().seed_primary_ppu_thread(
        OWNER,
        PpuThreadAttrs {
            entry: ENTRY,
            arg: 0,
            stack_base: 0x3_0000,
            stack_size: 0x1_0000,
            priority: 1000,
            tls_base: TLS as u32,
        },
    );
    let env = ModuleStartEnv {
        space: AddressSpaceId::BOOT,
        thread_owner: OWNER,
        pid: None,
        tls: TlsArea::above(TLS).expect("the TLS area is in range"),
        stack_pointer: 0x3_F000,
        break_pc: None,
        dump_mem_fault_ranges: Vec::new(),
        run_hle_stubbed: false,
        sink: Rc::new(crate::NullSink),
        ppu_tap: None,
        profile_mode,
    };
    let module = PrxLoadInfo {
        name: "profile_probe".to_string(),
        stem: "profile_probe".to_string(),
        base: ENTRY,
        data_end: TLS,
        toc: ENTRY + 0x8000,
        relocs_applied: 0,
        module_start: Some(LoadedOpd {
            code: ENTRY,
            toc: ENTRY + 0x8000,
        }),
        module_stop: None,
    };
    assert_eq!(
        run_module_start(&mut rt, &module, &env).expect("the module_start returns"),
        ModuleStartOutcome::Completed { steps: 3 }
    );
    rt.registry()
        .iter()
        .filter_map(|(_, unit)| unit.as_any().downcast_ref::<PpuExecutionUnit>())
        .map(|unit| unit.operand_profile().retired())
        .sum()
}

#[test]
fn a_profiled_boot_counts_what_module_start_retired() {
    assert_eq!(profiled_retirements(true), 2, "li and blr");
}

#[test]
fn an_unprofiled_boot_counts_nothing() {
    assert_eq!(profiled_retirements(false), 0);
}
