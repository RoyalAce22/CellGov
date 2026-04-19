//! CLI-layer support modules.
//!
//! Dependency direction (leaf first):
//!   exit  <- args, title, scenarios, compare, explore, dump, boot_cmd
//!   args  <- title, compare, explore, dump, boot_cmd
//!   scenarios  <- compare, explore, dump
//!   title  <- boot_cmd
//!   compare  <- explore   (load_baselines_from_dir reused)
//!
//! main.rs pulls from here by name; every module under `cli` is
//! `pub(crate)`-visible only within this binary.

pub(crate) mod args;
pub(crate) mod boot_cmd;
pub(crate) mod compare;
pub(crate) mod dump;
pub(crate) mod exit;
pub(crate) mod explore;
pub(crate) mod scenarios;
pub(crate) mod title;
