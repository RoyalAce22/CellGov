//! `explore` subcommand: bounded schedule exploration over a testkit
//! scenario, an LV2-driven ELF microtest, or a window of a composed
//! title boot.

mod dispatch;
mod scenario;
mod title;
mod window;

pub(crate) use dispatch::run;
