use super::*;
use cellgov_boot::kernel::KernelHypercall;
use cellgov_exec::UnitStatus;

const PUP_355: &str = "334e60a4ef5843a688c1c6aebf0951c3259429233c7a5aca5a24f0edad78a192";

fn spinning_kernel() -> KernelPosition {
    KernelPosition {
        pc: 0x8000_0000_0026_54e8,
        msr: 0x8000_0000_0000_8032,
        status: Some(UnitStatus::Runnable),
        last_hypercall: Some(KernelHypercall {
            number: 31,
            pc: 0x8000_0000_0000_807c,
            step: 64420,
        }),
        recent_pcs: vec![0x8000_0000_0026_54e0, 0x8000_0000_0026_54e8],
    }
}

#[test]
fn the_cap_clause_names_the_threads_position_or_says_it_has_none() {
    assert_eq!(
        render_position(0, Some(&spinning_kernel())),
        "hardware thread 0 is at pc 0x80000000002654e8 with MSR 0x8000000000008032 and status \
         Runnable; its last hypercall was 31 did_update_interrupt_mask at pc 0x800000000000807c \
         in step 64420; its last 2 steps ended at pc 0x80000000002654e0 0x80000000002654e8"
    );
    assert_eq!(
        render_position(1, None),
        "the run gives no state for hardware thread 1"
    );
}

#[test]
fn a_census_cut_off_by_its_cap_carries_both_threads_positions() {
    let unstepped = KernelPosition {
        pc: 0x100,
        last_hypercall: None,
        recent_pcs: Vec::new(),
        ..spinning_kernel()
    };
    let boot = KernelRun {
        steps: 100,
        stop: KernelRunStop::StepCap,
        repository_reads: Vec::new(),
        console: Vec::new(),
        thread0: Some(spinning_kernel()),
        thread1: Some(unstepped.clone()),
    };
    let Err(RepositoryCensusError::StepCap { threads, .. }) = census_rows(PUP_355, 100, &boot)
    else {
        panic!("a capped boot is not a census end");
    };
    assert_eq!(*threads, [Some(spinning_kernel()), Some(unstepped)]);
}

#[test]
fn the_cap_clause_says_no_step_for_an_empty_trail_and_names_one_step_in_the_singular() {
    let unstepped = KernelPosition {
        pc: 0x8000_0000_0000_0100,
        msr: 0x8000_0000_0000_0000,
        status: Some(UnitStatus::Runnable),
        last_hypercall: None,
        recent_pcs: Vec::new(),
    };
    assert_eq!(
        render_position(0, Some(&unstepped)),
        "hardware thread 0 is at pc 0x8000000000000100 with MSR 0x8000000000000000 and status \
         Runnable; it made no hypercall; it took no step"
    );
    let one_step = KernelPosition {
        recent_pcs: vec![0x8000_0000_0000_0104],
        ..unstepped
    };
    assert_eq!(
        render_position(0, Some(&one_step)),
        "hardware thread 0 is at pc 0x8000000000000100 with MSR 0x8000000000000000 and status \
         Runnable; it made no hypercall; its last step ended at pc 0x8000000000000104"
    );
}

#[test]
fn the_cap_clause_gives_only_the_number_of_a_hypercall_no_header_lists() {
    // A row number is a u16, so 0x1_001f has no row even though its low
    // 16 bits are 31, the number of did_update_interrupt_mask.
    let unlisted = KernelPosition {
        last_hypercall: Some(KernelHypercall {
            number: 0x1_001f,
            pc: 0x8000_0000_0000_807c,
            step: 7,
        }),
        ..spinning_kernel()
    };
    assert_eq!(
        render_position(0, Some(&unlisted)),
        "hardware thread 0 is at pc 0x80000000002654e8 with MSR 0x8000000000008032 and status \
         Runnable; its last hypercall was 65567 at pc 0x800000000000807c in step 7; its last 2 \
         steps ended at pc 0x80000000002654e0 0x80000000002654e8"
    );
}
