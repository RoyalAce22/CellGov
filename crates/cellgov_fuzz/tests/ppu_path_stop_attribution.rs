//! Tests full fault and syscall stop attribution between PPU execution paths.

use cellgov_fuzz::ppu_paths::{first_path_divergence, run_all_paths, PpuExecutionPath};
use cellgov_fuzz::ppu_reference::{
    compare_reference, parse_reference_json, PpuReferenceComponent, ReferenceField,
};
use cellgov_ppu::observation::PpuObservationComponent;
use cellgov_ppu::state::PpuState;

fn li(rt: u32, value: u16) -> u32 {
    (14 << 26) | (rt << 21) | u32::from(value)
}

fn lwz(rt: u32, ra: u32) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16)
}

#[test]
fn fault_site_registers_and_effective_address_survive_batch_rollback() {
    let mut state = PpuState::new();
    state.set_gpr(6, 0x2000_0000);
    state.set_lr(0x1234);
    let runs = run_all_paths(&[li(3, 7), lwz(4, 6)], &state, &[0; 64])
        .expect("fault sequence must be observable");

    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    for run in &runs {
        assert_eq!(run.observation.state.gpr[3], 0);
        assert_eq!(run.stop.diagnostics.faulting_ea, Some(0x2000_0000));
        assert_eq!(run.stop.diagnostics.lr, Some(0x1234));
        assert_eq!(
            run.stop
                .diagnostics
                .fault_regs
                .as_ref()
                .map(|regs| regs.gprs[3]),
            Some(7)
        );
        assert_eq!(run.stop.syscall_args, None);
    }
}

#[test]
fn syscall_lev_return_address_and_raw_arguments_agree() {
    let mut state = PpuState::new();
    state.set_lr(0xabcd);
    state.set_gpr(11, 0x77);
    state.set_gpr(3, 0x55);
    let runs = run_all_paths(&[0x4400_0022], &state, &[0; 64])
        .expect("LEV one syscall must be observable");

    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    for run in &runs {
        assert_eq!(run.stop.diagnostics.syscall_lev, Some(1));
        assert_eq!(run.stop.diagnostics.lr, Some(0xabcd));
        assert_eq!(
            run.stop.syscall_args.map(|args| args[..2].to_vec()),
            Some(vec![0x77, 0x55])
        );
        assert!(run.stop.diagnostics.fault_regs.is_none());
    }
}

#[test]
fn a_seeded_fault_site_diagnostic_difference_names_the_exact_paths() {
    let mut state = PpuState::new();
    state.set_gpr(6, 0x2000_0000);
    let mut runs =
        run_all_paths(&[lwz(3, 6)], &state, &[0; 64]).expect("fault sequence must be observable");
    runs[2].stop.diagnostics.faulting_ea = Some(0x2000_0004);

    let divergence = first_path_divergence(&runs).expect("fault address leak must be visible");

    assert_eq!(divergence.left, PpuExecutionPath::Plain);
    assert_eq!(divergence.right, PpuExecutionPath::Quickened);
    assert!(divergence.stop_differs);
    assert!(!divergence
        .observation
        .contains(&PpuObservationComponent::State));
}

#[test]
fn independent_reference_can_compare_represented_syscall_diagnostics() {
    let fixture = include_str!("fixtures/ppu_reference/li_r3_7_v1.json");
    let mut reference = parse_reference_json(fixture).expect("committed reference must parse");
    let mut state = PpuState::new();
    state.set_lr(0xabcd);
    state.set_gpr(11, 0x77);
    let runs = run_all_paths(&[0x4400_0022], &state, &[0; 64]).expect("syscall must be observable");
    reference.expected.stop.lr = ReferenceField::Value {
        value: Some(0xabcd),
    };
    reference.expected.stop.syscall_lev = ReferenceField::Value { value: Some(1) };
    reference.expected.stop.syscall_args = ReferenceField::Value {
        value: Some([0, 0, 0, 0, 0, 0, 0, 0, 0].to_vec()),
    };

    let comparison = compare_reference(&reference.expected, &runs[0]);

    assert!(comparison.compared.contains(&PpuReferenceComponent::StopLr));
    assert!(comparison
        .compared
        .contains(&PpuReferenceComponent::StopSyscallLev));
    assert!(comparison
        .compared
        .contains(&PpuReferenceComponent::StopSyscallArgs));
    assert!(comparison
        .differences
        .iter()
        .any(|difference| difference.field == PpuReferenceComponent::StopSyscallArgs));
}
