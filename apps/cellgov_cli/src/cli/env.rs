//! CLI environment-variable parsing helpers.

use super::args::CliArgError;
use super::exit::die;

/// Strict boolean parse for a `CELLGOV_*` env var. Unset and empty
/// both read as `false`. Any other value dies with a named diagnostic
/// so a stale shell setting cannot silently enable instrumentation.
pub(crate) fn parse_env_bool(name: &str) -> bool {
    parse_env_bool_inner(name, std::env::var(name).ok()).unwrap_or_else(|e| die(&e.to_string()))
}

fn parse_env_bool_inner(name: &str, value: Option<String>) -> Result<bool, CliArgError> {
    let Some(v) = value else {
        return Ok(false);
    };
    match v.trim().to_ascii_lowercase().as_str() {
        "" | "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        other => Err(CliArgError::EnvBoolUnknown {
            name: name.to_string(),
            got: other.to_string(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_env_bool_unset_is_false() {
        assert!(!parse_env_bool_inner("X", None).unwrap());
    }

    #[test]
    fn parse_env_bool_empty_is_false() {
        assert!(!parse_env_bool_inner("X", Some(String::new())).unwrap());
    }

    #[test]
    fn parse_env_bool_accepts_truthy_vocab() {
        for v in ["1", "true", "TRUE", "yes", "Yes", "on", "ON"] {
            assert!(
                parse_env_bool_inner("X", Some(v.to_string())).unwrap(),
                "{v:?} should be truthy"
            );
        }
    }

    #[test]
    fn parse_env_bool_accepts_falsy_vocab() {
        for v in ["0", "false", "FALSE", "no", "off"] {
            assert!(
                !parse_env_bool_inner("X", Some(v.to_string())).unwrap(),
                "{v:?} should be falsy"
            );
        }
    }

    #[test]
    fn parse_env_bool_rejects_garbage() {
        let err = parse_env_bool_inner("X", Some("maybe".to_string()))
            .unwrap_err()
            .to_string();
        assert!(err.contains("X="), "got: {err}");
        assert!(err.contains("maybe"), "got: {err}");
    }
}
