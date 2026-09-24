//! Bridges `Lv2Request`/`Lv2Dispatch` to `Runtime` mutation: classify a
//! syscall yield, route it through `Lv2Host::dispatch`, and fold the
//! result back into syscall responses, registry status, and mailbox
//! state. `handle_ppu_thread_create` lives in `ppu_create.rs`.

mod checks;
mod effects;
mod handlers;
mod syscall;

pub(crate) use checks::check_response_updates;

#[cfg(test)]
#[path = "tests/lv2_dispatch_tests.rs"]
mod tests;
