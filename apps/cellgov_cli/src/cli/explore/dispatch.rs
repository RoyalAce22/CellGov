//! Which of the three `explore` targets an invocation named.

use std::path::Path;

use super::{scenario, title};
use crate::cli::exit::die;
use crate::cli::parse::{ExploreArgs, ExploreCommand, OutputFormat};
use crate::cli::scenarios::{scenario_factory, MICROTESTS};

pub(crate) fn run(
    args: &ExploreArgs,
    format: OutputFormat,
    scenarios_list: &[&str],
    vfs_flag: Option<&Path>,
) {
    match (&args.command, &args.scenario) {
        (Some(ExploreCommand::Title(title_args)), _) => {
            title::run(title_args, format, vfs_flag);
        }
        (
            Some(ExploreCommand::Micro {
                name,
                observations_dir,
            }),
            _,
        ) => {
            if !MICROTESTS.contains(&name.as_str()) {
                die(&format!(
                    "unknown microtest: {name}\navailable: {}",
                    MICROTESTS.join(", ")
                ));
            }
            match observations_dir {
                Some(dir) => {
                    scenario::run_explore_micro_oracle(name, &dir.display().to_string(), format);
                }
                None => scenario::run_explore_micro(name, format),
            }
        }
        (None, Some(target)) => match scenario_factory(target) {
            Some(factory) => scenario::run_explore(&factory, target, format),
            None => die(&format!(
                "unknown scenario: {target}\navailable: {}",
                scenarios_list.join(", ")
            )),
        },
        // clap requires the positional unless a subcommand is present,
        // so no argv reaches this arm.
        (None, None) => die("explore: no scenario, micro-test or title named"),
    }
}
