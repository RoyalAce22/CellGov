//! Firmware PRX loading, module_start execution, and TLS pre-init
//! for `run-game`.

mod got;
mod load;
mod module_start;
mod tls;
mod types;

pub(super) use load::{install_unresolved_trampolines_only, load_firmware_set_bound};
pub(super) use module_start::run_module_start;
pub(super) use tls::{pre_init_tls, TLS_BASE};
// Re-exported so callers in `game/` can name the type directly even
// though current call sites only see it through return-type inference.
#[allow(unused_imports)]
pub(super) use types::PrxLoadInfo;
