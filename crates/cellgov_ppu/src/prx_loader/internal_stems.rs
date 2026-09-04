//! The `sys/internal/` stem list; see [`FIRMWARE_INTERNAL_PRX_STEMS`].

/// Modules under `sys/internal/` that the system shell loads by full path.
///
/// Import-closure selection cannot derive these stems. The shell names
/// them by filesystem path from its own runtime data, so a firmware-exec
/// boot adds them to the candidate set explicitly.
pub const FIRMWARE_INTERNAL_PRX_STEMS: &[&str] = &["libfs_utility2"];

#[cfg(test)]
#[path = "tests/internal_stems_tests.rs"]
mod tests;
