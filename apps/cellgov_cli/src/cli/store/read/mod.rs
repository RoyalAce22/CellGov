//! The store's read surface: `status`, and the `list` / `show` /
//! `verify` verbs under `firmware` and `title`.
//!
//! Results go to stdout -- one JSON document under `--format json`,
//! aligned columns otherwise. `firmware kernels` also refreshes its
//! operator-local coverage report; no command here changes an installed
//! artefact. Warnings and hints go to stderr.

mod collect;
mod kernels;
mod list;
pub(crate) mod model;
mod pups;
mod render;
mod size;
mod status;
mod verify;
mod view;

pub(crate) use kernels::firmware_kernels;
pub(crate) use list::{firmware_list, firmware_show, title_list, title_show};
pub(crate) use pups::firmware_verify_pups;
pub(crate) use status::status;
pub(crate) use verify::{firmware_verify, title_verify};
pub(crate) use view::store_root;

use render::{emit, human_bytes, key_list};
use size::tree_bytes;
use view::view;

#[cfg(test)]
#[path = "tests/read_tests.rs"]
mod tests;
