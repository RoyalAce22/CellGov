use std::cell::RefCell;
use std::rc::Rc;

use cellgov_effects::Effect;
use cellgov_event::UnitId;
use cellgov_exec::{
    ExecutionContext, ExecutionStepResult, ExecutionUnit, LocalDiagnostics, SaveRestore,
    UnitStatus, YieldReason,
};
use cellgov_lv1::{
    page_table_storage, HypercallDisposition, HypercallRefusal, HypercallRequest,
    PAGE_TABLE_STORAGE_BASE,
};
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::lv1::address_space::game_os_boot_vas_id;
use cellgov_ps3_abi::lv1::call::{CONSTRUCT_VIRTUAL_ADDRESS_SPACE, SELECT_VIRTUAL_ADDRESS_SPACE};
use cellgov_time::{Budget, GuestTicks, InstructionCost};

use crate::runtime::types::StepError;
use crate::runtime::Runtime;

/// The address of the scripted `sc`.
const PC: u64 = 0x100;

/// The SDR1 and HID6\[LB\] of each page table load the unit took, and the
/// r3 its step after the call received.
#[derive(Debug, Default, PartialEq, Eq)]
struct Seen {
    loads: Vec<(u64, u8)>,
    r3: Option<Option<u64>>,
}

/// A unit that selects the boot address space, then records what its
/// next step receives and finishes. `translation` tells whether it takes
/// a page table load.
#[derive(Clone)]
struct Selector {
    id: UnitId,
    translation: bool,
    steps: u32,
    seen: Rc<RefCell<Seen>>,
}

impl ExecutionUnit for Selector {
    type Snapshot = ();
    fn unit_id(&self) -> UnitId {
        self.id
    }
    fn status(&self) -> UnitStatus {
        if self.steps >= 2 {
            UnitStatus::Finished
        } else {
            UnitStatus::Runnable
        }
    }
    fn ppe_thread(&self) -> Option<u8> {
        Some(0)
    }
    fn system_call_save_restore(&self) -> Option<SaveRestore> {
        Some(SaveRestore {
            srr0: PC + 4,
            srr1: 0,
        })
    }
    fn load_page_table(&mut self, sdr1: u64, lb: u8) -> bool {
        self.seen.borrow_mut().loads.push((sdr1, lb));
        self.translation
    }
    fn takes_page_table(&self) -> bool {
        self.translation
    }
    fn run_until_yield(
        &mut self,
        _budget: Budget,
        ctx: &ExecutionContext<'_>,
        _effects: &mut Vec<Effect>,
    ) -> ExecutionStepResult {
        self.steps += 1;
        if self.steps == 1 {
            let mut args = [0; 9];
            args[0] = SELECT_VIRTUAL_ADDRESS_SPACE;
            args[1] = vas_id();
            return ExecutionStepResult {
                yield_reason: YieldReason::Syscall,
                consumed_cost: InstructionCost::new(3),
                local_diagnostics: LocalDiagnostics::with_pc_lr_syscall_lev(PC, 0, 1),
                fault: None,
                syscall_args: Some(args),
            };
        }
        self.seen.borrow_mut().r3 = Some(ctx.syscall_return());
        ExecutionStepResult {
            yield_reason: YieldReason::Finished,
            consumed_cost: InstructionCost::new(1),
            local_diagnostics: LocalDiagnostics::empty(),
            fault: None,
            syscall_args: None,
        }
    }
    fn snapshot(&self) {}
}

/// The vas id that the 4.93 hypervisor gives the kernel's address space.
fn vas_id() -> u64 {
    game_os_boot_vas_id("4.93").expect("a 4.93 boot vas id")
}

/// A 4.93 runtime over the page table storage whose hypervisor holds the
/// kernel's address space, with one [`Selector`].
fn runtime(translation: bool) -> (Runtime, Rc<RefCell<Seen>>) {
    let memory = GuestMemory::from_regions(vec![page_table_storage()]).expect("storage");
    let mut rt = Runtime::new(memory.clone(), Budget::new(4), 100);
    rt.set_hypervisor_firmware("4.93");
    // The 4.93 kernel's construction
    // [OFW-4.93 lv2_kernel a:0x800000000005dcc0-0x800000000005dcf4].
    let mut inputs = [0; 8];
    inputs[1] = 2;
    inputs[2] = 0x1014_0000_0000_0000;
    let construct = HypercallRequest {
        unit: UnitId::new(0),
        number: CONSTRUCT_VIRTUAL_ADDRESS_SPACE,
        inputs,
        pc: 0,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::ZERO,
    };
    let constructed = rt.hypervisor.dispatch_with(&construct, &memory);
    assert!(
        matches!(constructed, HypercallDisposition::Answered(_)),
        "{constructed:?}"
    );
    let seen = Rc::new(RefCell::new(Seen::default()));
    let shared = Rc::clone(&seen);
    rt.register_unit_with(move |id| Selector {
        id,
        translation,
        steps: 0,
        seen: shared,
    });
    (rt, seen)
}

fn step(rt: &mut Runtime) -> Result<(), StepError> {
    let mut s = rt.step()?;
    rt.commit_step(&mut s).expect("commit");
    Ok(())
}

#[test]
fn a_selection_from_a_unit_that_refuses_the_load_is_refused_and_selects_nothing() {
    let (mut rt, seen) = runtime(false);
    step(&mut rt).expect("the hypercall step");
    assert_eq!(step(&mut rt), Err(StepError::HypercallRefused));
    assert_eq!(
        rt.hypervisor().refusal(),
        Some(HypercallRefusal::UnmodelledInput {
            number: SELECT_VIRTUAL_ADDRESS_SPACE,
            unit: UnitId::new(0),
            pc: PC,
            what: "a selection from a unit that holds no SDR1 or HID6 for the page table load",
        })
    );
    assert_eq!(rt.hypervisor().address_spaces().selected(), None);
    assert_eq!(*seen.borrow(), Seen::default());
}

#[test]
fn a_selection_from_a_unit_that_takes_the_load_loads_it_and_answers_success() {
    let (mut rt, seen) = runtime(true);
    step(&mut rt).expect("the hypercall step");
    step(&mut rt).expect("the caller's next step");
    assert_eq!(rt.hypervisor().refusal(), None);
    assert_eq!(rt.hypervisor().address_spaces().selected(), Some(vas_id()));
    assert_eq!(
        *seen.borrow(),
        Seen {
            // HTABSIZE 0 for the 256 KB table, and LB for 64 KB and 1 MB pages.
            loads: vec![(PAGE_TABLE_STORAGE_BASE, 0b1001)],
            r3: Some(Some(0)),
        }
    );
}
