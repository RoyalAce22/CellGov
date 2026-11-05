//! Firmware PRX loading, module_start execution, and TLS pre-init.

mod got;
mod load;
mod module_start;
mod tls;
mod types;

#[cfg(test)]
#[path = "tests/code_span_tests.rs"]
mod code_span_tests;

pub use load::{
    install_unresolved_trampolines_only, load_firmware_set_bound, load_firmware_set_from,
    FirmwareCandidates, FirmwareLoadError,
};
pub(crate) use module_start::run_module_start_batch;
pub use module_start::{
    run_module_start, ModuleStartEnv, ModuleStartError, ModuleStartOutcome, PER_MODULE_STEP_BUDGET,
};
pub use tls::{install_kernel_context_opd, pre_init_tls, TlsArea, TlsError};
pub use types::{
    HostLinkMaps, PrxLoadInfo, PrxLoadStageError, UnresolvedRequesters, VerifiedFirmware,
};
