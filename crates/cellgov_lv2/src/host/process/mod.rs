//! `sys_process` dispatch handlers and per-class active-object
//! counters. Dispatch methods live in [`dispatch`]; the counter
//! side-table lives in [`counts`].

mod counts;
mod dispatch;

pub(in crate::host) use counts::ProcessCounts;
