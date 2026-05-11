//! `sys_fs_*` host dispatch.
//!
//! Each handler lives in its own submodule; shared constants and
//! helpers (open-flag family, stat wire format, path scan, mount
//! resolver, NULL/alignment guard) live in their typed homes below.
//!
//! The `Lv2Host` type itself stays in `crate::host`; this directory
//! only contributes additional `impl Lv2Host { ... }` blocks scoped
//! to the FS surface.

mod close;
mod closedir;
mod flags;
mod lseek;
mod mount;
mod open;
mod opendir;
mod path;
mod ptr;
mod read;
mod readdir;
mod stat;
mod stat_layout;

#[cfg(test)]
mod tests;
