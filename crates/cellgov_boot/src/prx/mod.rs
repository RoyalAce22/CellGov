//! Firmware PRX loading, module_start execution, and TLS pre-init.

mod got;
mod load;
mod module_start;
mod tls;
mod types;

pub use load::{
    install_unresolved_trampolines_only, load_firmware_set_bound, load_firmware_set_from,
    FirmwareCandidates, FirmwareLoadError,
};
pub use module_start::{
    run_module_start, ModuleStartEnv, ModuleStartError, ModuleStartOutcome, PER_MODULE_STEP_BUDGET,
};
pub use tls::{install_kernel_context_opd, pre_init_tls, TlsError, TLS_BASE};
pub use types::{
    HostLinkMaps, PrxLoadInfo, PrxLoadStageError, UnresolvedRequesters, VerifiedFirmware,
};
