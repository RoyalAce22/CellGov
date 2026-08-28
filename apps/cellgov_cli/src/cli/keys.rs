//! The CLI's key vault: loaded on the first SELF that needs it, once.

use cellgov_install::keys::{KeyVault, KeyVaultError};
use cellgov_install::self_image::is_sce_wrapped;

static NO_KEYS: KeyVault = KeyVault::empty();

/// The vault `bytes` decrypts under: the operator's for an SCE
/// wrapper, an empty one for a plaintext image; a vault that will not
/// load dies naming the cause.
pub(crate) fn key_vault_for(bytes: &[u8]) -> &'static KeyVault {
    try_key_vault_for(bytes).unwrap_or_else(|e| super::exit::die(&format!("key vault: {e}")))
}

/// [`key_vault_for`] returning the load refusal instead of dying, for
/// a loader that runs inside `Runtime::step`.
pub(crate) fn try_key_vault_for(bytes: &[u8]) -> Result<&'static KeyVault, &'static KeyVaultError> {
    if is_sce_wrapped(bytes) {
        key_vault()
    } else {
        Ok(&NO_KEYS)
    }
}

/// The vault every SELF open in this process decrypts under, loaded
/// on first use; a load refusal is kept and answered the same way to
/// every later caller.
#[cfg(feature = "decrypt")]
fn key_vault() -> Result<&'static KeyVault, &'static KeyVaultError> {
    use std::sync::OnceLock;

    static VAULT: OnceLock<Result<KeyVault, KeyVaultError>> = OnceLock::new();
    VAULT.get_or_init(KeyVault::load).as_ref()
}

/// Without the `decrypt` feature the vault is never consulted: an
/// SCE-wrapped image is refused by the feature before any key lookup.
#[cfg(not(feature = "decrypt"))]
fn key_vault() -> Result<&'static KeyVault, &'static KeyVaultError> {
    Ok(&NO_KEYS)
}
