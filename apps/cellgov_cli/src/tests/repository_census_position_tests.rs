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
fn the_cap_clause_names_the_kernels_position_or_says_it_has_none() {
    assert_eq!(
        render_position(Some(&spinning_kernel())),
        "the kernel unit is at pc 0x80000000002654e8 with MSR 0x8000000000008032 and status \
         Runnable; its last hypercall was 31 did_update_interrupt_mask at pc 0x800000000000807c \
         in step 64420; its last 2 steps ended at pc 0x80000000002654e0 0x80000000002654e8"
    );
    assert_eq!(
        render_position(None),
        "the run gives no state for the kernel unit"
    );
}

#[test]
fn a_census_cut_off_by_its_cap_carries_the_kernels_position() {
    let boot = KernelRun {
        steps: 100,
        stop: KernelRunStop::StepCap,
        repository_reads: Vec::new(),
        console: Vec::new(),
        kernel: Some(spinning_kernel()),
    };
    let Err(RepositoryCensusError::StepCap { kernel, .. }) = census_rows(PUP_355, 100, &boot)
    else {
        panic!("a capped boot is not a census end");
    };
    assert_eq!(kernel, Some(spinning_kernel()));
}

#[test]
fn the_cap_clause_says_the_kernel_took_no_step_when_the_cap_fell_before_its_first() {
    let unstepped = KernelPosition {
        pc: 0x8000_0000_0000_0100,
        msr: 0x8000_0000_0000_0000,
        status: Some(UnitStatus::Runnable),
        last_hypercall: None,
        recent_pcs: Vec::new(),
    };
    assert_eq!(
        render_position(Some(&unstepped)),
        "the kernel unit is at pc 0x8000000000000100 with MSR 0x8000000000000000 and status \
         Runnable; it made no hypercall; it took no step"
    );
    let one_step = KernelPosition {
        recent_pcs: vec![0x8000_0000_0000_0104],
        ..unstepped
    };
    let clause = render_position(Some(&one_step));
    assert!(
        clause.ends_with("; its last step ended at pc 0x8000000000000104"),
        "{clause}"
    );
}
