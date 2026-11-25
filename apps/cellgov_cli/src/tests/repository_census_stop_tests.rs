use super::*;
use cellgov_effects::FaultKind;
use cellgov_lv1::PausedThread;

#[test]
fn a_pause_of_every_thread_names_each_thread_and_its_decrementer_wake() {
    let stop = KernelRunStop::NoWake {
        paused: vec![
            PausedThread {
                thread: 0,
                decrementer_wakes: false,
            },
            PausedThread {
                thread: 1,
                decrementer_wakes: true,
            },
        ],
    };
    assert_eq!(
        stop_text(&stop),
        "a pause of every hardware thread that no wake CellGov delivers ends: thread 0 (no \
         decrementer wake), thread 1 (its decrementer wakes it)"
    );
}

#[test]
fn a_fault_names_the_hardware_thread_that_faulted() {
    let fault = FaultKind::Guest(0x0000_0007);
    let at = KernelRunStop::Fault {
        thread: 1,
        fault,
        pc: Some(0x110),
    };
    assert_eq!(
        stop_text(&at),
        format!("a fault {fault:?} of hardware thread 1 at pc 0x0000000000000110")
    );
    let nowhere = KernelRunStop::Fault {
        thread: 0,
        fault,
        pc: None,
    };
    assert_eq!(
        stop_text(&nowhere),
        format!("a fault {fault:?} of hardware thread 0")
    );
}

fn parked_at(pc: u64) -> KernelPosition {
    KernelPosition {
        pc,
        msr: 0,
        status: None,
        last_hypercall: None,
        recent_pcs: Vec::new(),
    }
}

#[test]
fn the_cap_message_names_thread_0_then_thread_1_each_at_its_own_pc() {
    let error = RepositoryCensusError::StepCap {
        max_steps: 10,
        steps: 10,
        reads: 0,
        threads: Box::new([Some(parked_at(0x100)), Some(parked_at(0x200))]),
    };
    assert_eq!(
        error.to_string(),
        "the boot reached the step cap of 10 after 10 steps and 0 repository reads, before \
         CellGov's hypervisor stopped it; hardware thread 0 is at pc 0x0000000000000100 with \
         MSR 0x0000000000000000 and status none; it made no hypercall; it took no step; \
         hardware thread 1 is at pc 0x0000000000000200 with MSR 0x0000000000000000 and status \
         none; it made no hypercall; it took no step; raise --max-steps"
    );
}

#[test]
fn a_pause_that_no_wake_ends_is_a_census_end_that_keeps_its_reads() {
    let key = cellgov_ps3_abi::lv1::repository::RepositoryKey {
        lpar: 2,
        n1: 0x6269_0000,
        n2: 0x7267_6e74_6f74_616c,
        n3: 0,
        n4: 0,
    };
    let boot = KernelRun {
        steps: 10,
        stop: KernelRunStop::NoWake {
            paused: vec![PausedThread {
                thread: 0,
                decrementer_wakes: true,
            }],
        },
        repository_reads: vec![cellgov_lv1::RepositoryRead {
            key,
            values: Some((1, 0)),
            unit: cellgov_event::UnitId::new(1),
            pc: 0x8000_0000_0000_ce3c,
        }],
        console: Vec::new(),
        thread0: Some(parked_at(0x100)),
        thread1: Some(parked_at(0x200)),
        hypercall_sites: Default::default(),
    };
    assert_eq!(
        census_rows("00", 10, &boot).expect("a pause is where CellGov stops the boot"),
        vec![RepositoryReadRow {
            pup_sha256: "00".to_string(),
            key,
            node: key.path(),
            first: 0,
            answered: true,
        }]
    );
}
