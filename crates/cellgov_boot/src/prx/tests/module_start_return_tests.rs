//! The module_start loop reads an instruction storage interrupt at PC 0
//! with LR 0 as the module_start's return, and any other fault as a
//! fault of the module.

use std::rc::Rc;

use cellgov_core::{AddressSpaceId, Runtime};
use cellgov_event::UnitId;
use cellgov_lv2::PpuThreadAttrs;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ppu::sprx::LoadedOpd;
use cellgov_time::Budget;

use super::{run_module_start, ModuleStartEnv, ModuleStartError, ModuleStartOutcome, PrxLoadInfo};
use crate::prx::TlsArea;

/// The thread the module_start unit runs as. No unit holds this id, so
/// the module_start unit is the only one the scheduler runs.
const OWNER: UnitId = UnitId::new(7);
/// The module_start entry point, and the floor below which
/// [`AtZero::Unmapped`] unmaps the memory.
const ENTRY: u64 = 0x1_0000;
/// The base of the TLS area, clear of the entry.
const TLS: u64 = 0x2_0000;
/// `blr`.
const BLR: u32 = 0x4E80_0020;
/// `bla 0`: branch to 0 and set LR to the address after the branch.
const BLA_0: u32 = 0x4800_0003;
/// `tw 31,r0,r0`: a trap whose condition always holds.
const TW_ALWAYS: u32 = 0x7FE0_0008;

/// What lies at address 0.
#[derive(Clone, Copy)]
enum AtZero {
    /// No word: as in a title's memory after the boot, the memory below
    /// the entry is unmapped, and the entry word is the only code.
    Unmapped,
    /// This word, in mapped memory that fetches at every address.
    Word(u32),
}

/// Runs a module_start whose first word is `entry_word`.
fn run(entry_word: u32, at_zero: AtZero) -> Result<ModuleStartOutcome, ModuleStartError> {
    let mut mem = GuestMemory::new(0x4_0000);
    let entry = ByteRange::new(GuestAddr::new(ENTRY), 4).unwrap();
    mem.apply_commit(entry, &entry_word.to_be_bytes()).unwrap();
    match at_zero {
        AtZero::Unmapped => {
            mem.declare_code(ENTRY, 4);
            mem.unmap_below(ENTRY);
            mem.restrict_fetch();
        }
        AtZero::Word(word) => {
            let zero = ByteRange::new(GuestAddr::new(0), 4).unwrap();
            mem.apply_commit(zero, &word.to_be_bytes()).unwrap();
        }
    }
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
    };
    let module = PrxLoadInfo {
        name: "sentinel_probe".to_string(),
        stem: "sentinel_probe".to_string(),
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
    run_module_start(&mut rt, &module, &env)
}

/// The guest code a module_start fault's detail opens with.
fn faulted_code(outcome: Result<ModuleStartOutcome, ModuleStartError>) -> String {
    match outcome {
        Err(ModuleStartError::Faulted { detail, .. }) => {
            detail.split(' ').next().unwrap_or_default().to_string()
        }
        other => panic!("the module_start did not fault: {other:?}"),
    }
}

#[test]
fn a_blr_to_the_lr_zero_sentinel_is_a_return() {
    assert_eq!(
        run(BLR, AtZero::Unmapped).expect("the module_start returns"),
        ModuleStartOutcome::Completed { steps: 2 }
    );
}

#[test]
fn an_unmapped_zero_reached_with_a_nonzero_lr_is_a_fault() {
    let not_found = cellgov_ppu::FAULT_INSTRUCTION_STORAGE | 0x4000;
    assert_eq!(
        faulted_code(run(BLA_0, AtZero::Unmapped)),
        format!("0x{not_found:08x}")
    );
}

#[test]
fn an_illegal_word_at_a_mapped_zero_reached_with_lr_zero_is_a_fault() {
    let illegal = cellgov_ppu::FAULT_PROGRAM | 0x0800;
    assert_eq!(
        faulted_code(run(BLR, AtZero::Word(0))),
        format!("0x{illegal:08x}")
    );
}

#[test]
fn a_trap_at_a_mapped_zero_reached_with_lr_zero_is_a_fault() {
    let trap_to_31 = cellgov_ppu::FAULT_PROGRAM | 0x0200 | 31;
    assert_eq!(
        faulted_code(run(BLR, AtZero::Word(TW_ALWAYS))),
        format!("0x{trap_to_31:08x}")
    );
}
