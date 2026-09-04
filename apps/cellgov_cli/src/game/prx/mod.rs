//! Firmware PRX loading, module_start execution, and TLS pre-init
//! for `boot run`.

mod got;
mod load;
mod module_start;
mod tls;
mod types;

pub(super) use load::{
    install_unresolved_trampolines_only, load_firmware_set_bound, load_firmware_set_from,
    FirmwareCandidates,
};
pub(super) use module_start::{
    run_module_start, ModuleStartEnv, ModuleStartError, ModuleStartOutcome,
};
pub(super) use tls::{install_kernel_context_opd, pre_init_tls, TLS_BASE};
pub(super) use types::{HostLinkMaps, PrxLoadInfo, VerifiedFirmware};
