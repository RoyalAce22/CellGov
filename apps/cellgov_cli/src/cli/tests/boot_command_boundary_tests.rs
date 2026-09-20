use super::*;

#[test]
fn an_environment_refusal_keeps_its_own_diagnostic_context() {
    let error = CompositionResolutionError::Command(CommandError::failed("environment"))
        .into_boot_command_error();
    assert_eq!(error.to_string(), "environment");
}

#[test]
fn a_composition_refusal_keeps_the_boot_context() {
    let error = CompositionResolutionError::Compose(ComposeError::FirmwareDirectory {
        path: "missing".to_string(),
    })
    .into_boot_command_error();
    assert_eq!(
        error.to_string(),
        "boot: --firmware-dir: missing is not an existing directory"
    );
}

#[test]
fn a_run_failure_keeps_the_boot_run_context() {
    assert_eq!(boot_run_error(&"failure").to_string(), "boot run: failure");
}

#[test]
fn a_child_interrupt_remains_a_command_error() {
    let error = separate_spawn_command_error(game::SpawnError::Command(CommandError::Interrupted))
        .expect_err("a child interrupt reaches the process boundary");
    assert!(matches!(error, CommandError::Interrupted));
}
