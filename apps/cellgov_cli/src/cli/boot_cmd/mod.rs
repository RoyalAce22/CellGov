//! Dispatch for the boot-family subcommands: `boot run`,
//! `boot bench-once`, and `boot bench`.

mod bench;
mod compose;
mod inputs;
mod run;

pub(crate) use bench::{bench_boot, bench_boot_once};
pub(super) use bench::{separate_spawn_command_error, EXIT_SPREAD_EXCEEDED};
pub(super) use compose::{
    anchor_plan, firmware_module_dir, plan_max_steps, resolve_composition, selection_args,
    try_resolve_composition, CompositionResolutionError,
};
pub(super) use inputs::{resolve_boot_inputs, try_resolve_cell_inputs, BootInputs};
pub(crate) use run::run_game;

#[cfg(test)]
#[path = "tests/boot_command_boundary_tests.rs"]
mod command_boundary_tests;
