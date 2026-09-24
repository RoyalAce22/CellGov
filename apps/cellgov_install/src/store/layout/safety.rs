//! Which strings a store path may carry as one component, and the Win32 device names it refuses.

/// Whether a string is safe to use as a single path component under a
/// store root: non-empty, no leading or trailing dot, no Win32 device
/// name, and `[A-Za-z0-9._-]` only.
///
/// A leading dot collides with the `.staging-*` / `.uninstalling-*`
/// residue sharing the directory. A trailing dot is dropped when Win32
/// normalizes a path component, so `4.91.` and `4.91` would be two keys
/// naming one directory.
///
/// Win32 resolves a reserved device name to a character device. A
/// firmware version keyed `NUL` writes its record to `NUL.install.toml`,
/// which the null device discards and reads back empty.
pub(crate) fn is_safe_component(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('.')
        && !s.ends_with('.')
        && !is_reserved_device_name(s)
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

/// Whether `s` resolves to a Win32 character device.
///
/// The match is case-insensitive and covers the part before the first
/// dot. Every host refuses the name, so a record one platform writes is
/// a record the other reads.
fn is_reserved_device_name(s: &str) -> bool {
    let stem = s.split('.').next().unwrap_or_default();
    if ["CON", "PRN", "AUX", "NUL"]
        .iter()
        .any(|name| stem.eq_ignore_ascii_case(name))
    {
        return true;
    }
    // Ports number from 1: `COM0` and `LPT0` name no device.
    match stem.as_bytes() {
        [a, b, c, d] if d.is_ascii_digit() && *d != b'0' => {
            let head = [
                a.to_ascii_uppercase(),
                b.to_ascii_uppercase(),
                c.to_ascii_uppercase(),
            ];
            head == *b"COM" || head == *b"LPT"
        }
        _ => false,
    }
}

/// Whether a record `store_path` stays under the root it is resolved
/// against: non-empty, relative, `/`-separated, every component an
/// [`is_safe_component`] name.
///
/// Shared by [`StoreLayout::store_path_of`](super::StoreLayout::store_path_of) and the record parse gate,
/// so the writer and the reader cannot disagree about which paths are
/// expressible.
pub(crate) fn store_path_is_safe(path: &str) -> bool {
    !path.is_empty() && path.split('/').all(is_safe_component)
}
