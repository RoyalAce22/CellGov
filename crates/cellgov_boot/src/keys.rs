//! How a boot reaches the operator's key vault.

use cellgov_install::keys::{KeyVault, KeyVaultError};

/// Where a boot gets the vault an SCE-wrapped image decrypts under.
///
/// The vault is the operator's, and it loads once per process. A boot
/// only asks which vault covers the bytes in front of it. Plaintext
/// input answers with an empty vault, so a build or a machine with no
/// keys still boots a raw ELF.
///
/// Cross-module contract: the spawn loader runs inside `Runtime::step`
/// and cannot stop the process, so `vault_for` returns the refusal
/// instead. The implementation must answer the same way for the whole
/// run -- a vault that loads for the title and not for a child it
/// spawns would make the boot depend on spawn order.
pub trait KeyVaultSource {
    /// The vault `bytes` decrypts under, or why the operator's vault
    /// did not load.
    fn vault_for(&self, bytes: &[u8]) -> Result<&KeyVault, &KeyVaultError>;
}
