//! `sys_process` dispatch handlers, the process identity table, and
//! per-class active-object counters.

mod counts;
mod dispatch;
mod table;

pub(in crate::host) use counts::ProcessCounts;
pub use table::{ProcessEntry, ProcessTable};
