//! The environment variables the `cellgov` binary recognizes.

pub(crate) struct EnvVar {
    pub(crate) name: &'static str,
    pub(crate) scope: Scope,
    pub(crate) purpose: &'static str,
}

/// The audience for an environment variable.
#[derive(Clone, Copy)]
pub(crate) enum Scope {
    /// An operator sets this for ordinary command use.
    Operator,
    /// A developer sets this while diagnosing a run.
    Debug,
    /// A synthetic harness sets this variable.
    TestOnly,
}

impl Scope {
    fn label(self) -> &'static str {
        match self {
            Self::Operator => "operator",
            Self::Debug => "debug",
            Self::TestOnly => "test-only",
        }
    }
}

pub(crate) const PS3_VFS_ROOT: &str = "CELLGOV_PS3_VFS_ROOT";
pub(crate) const KEYS: &str = "CELLGOV_KEYS";
pub(crate) const NO_FIRMWARE_DIR: &str = "CELLGOV_NO_FIRMWARE_DIR";
pub(crate) const RUNGAME_PROFILE: &str = "CELLGOV_RUNGAME_PROFILE";
pub(crate) const OBS_NULL_SINK: &str = "CELLGOV_OBS_NULL_SINK";
pub(crate) const HLE_RETURN_WATCH: &str = "CELLGOV_HLE_RETURN_WATCH";
pub(crate) const HLE_RETURN_WATCH_PCS: &str = "CELLGOV_HLE_RETURN_WATCH_PCS";
pub(crate) const HLE_RETURN_WATCH_PATH: &str = "CELLGOV_HLE_RETURN_WATCH_PATH";
pub(crate) const STORE_WATCH: &str = "CELLGOV_STORE_WATCH";
pub(crate) const STORE_WATCH_PATH: &str = "CELLGOV_STORE_WATCH_PATH";
pub(crate) const VALUE_SAMPLE: &str = "CELLGOV_VALUE_SAMPLE";
pub(crate) const VALUE_SAMPLE_PATH: &str = "CELLGOV_VALUE_SAMPLE_PATH";
pub(crate) const VALUE_SAMPLE_STRIDE: &str = "CELLGOV_VALUE_SAMPLE_STRIDE";

const ENV_VARS: &[EnvVar] = &[
    EnvVar {
        name: KEYS,
        scope: Scope::Operator,
        purpose: "Override the key-vault file.",
    },
    EnvVar {
        name: PS3_VFS_ROOT,
        scope: Scope::Operator,
        purpose: "Override the PS3 VFS root.",
    },
    EnvVar {
        name: "CELLGOV_<TITLE_ID>_CONTENT_DIR",
        scope: Scope::Operator,
        purpose: "Override one title's installed content directory.",
    },
    EnvVar {
        name: "CELLGOV_NO_COLOR",
        scope: Scope::Operator,
        purpose: "Disable color for this program.",
    },
    EnvVar {
        name: "CELLGOV_FORCE_ANSI",
        scope: Scope::Operator,
        purpose: "Force ANSI terminal output.",
    },
    EnvVar {
        name: "CELLGOV_FW_DEBUG",
        scope: Scope::Debug,
        purpose: "Trace firmware package decryption.",
    },
    EnvVar {
        name: "CELLGOV_BOOT_TRACE_MEM",
        scope: Scope::Debug,
        purpose: "Record boot memory tracing.",
    },
    EnvVar {
        name: RUNGAME_PROFILE,
        scope: Scope::Debug,
        purpose: "Print host-time boot spans.",
    },
    EnvVar {
        name: HLE_RETURN_WATCH,
        scope: Scope::Debug,
        purpose: "Watch HLE return NIDs.",
    },
    EnvVar {
        name: HLE_RETURN_WATCH_PCS,
        scope: Scope::Debug,
        purpose: "Limit an HLE watch to PCs.",
    },
    EnvVar {
        name: HLE_RETURN_WATCH_PATH,
        scope: Scope::Debug,
        purpose: "Write HLE watch records to a file.",
    },
    EnvVar {
        name: STORE_WATCH,
        scope: Scope::Debug,
        purpose: "Watch guest stores in an address range.",
    },
    EnvVar {
        name: STORE_WATCH_PATH,
        scope: Scope::Debug,
        purpose: "Write store-watch records to a file.",
    },
    EnvVar {
        name: VALUE_SAMPLE,
        scope: Scope::Debug,
        purpose: "Sample guest values in an address range.",
    },
    EnvVar {
        name: VALUE_SAMPLE_PATH,
        scope: Scope::Debug,
        purpose: "Write value samples to a file.",
    },
    EnvVar {
        name: VALUE_SAMPLE_STRIDE,
        scope: Scope::Debug,
        purpose: "Set the value-sample stride.",
    },
    EnvVar {
        name: NO_FIRMWARE_DIR,
        scope: Scope::TestOnly,
        purpose: "Suppress the synthetic firmware-directory default.",
    },
    EnvVar {
        name: OBS_NULL_SINK,
        scope: Scope::TestOnly,
        purpose: "Discard observation output in a synthetic run.",
    },
    EnvVar {
        name: "CELLGOV_RETAIN_SCRATCH",
        scope: Scope::TestOnly,
        purpose: "Retain a scratch directory after a test.",
    },
];

pub(crate) fn all() -> &'static [EnvVar] {
    ENV_VARS
}

pub(crate) fn render() -> String {
    let mut table = String::from("| Variable | Scope | Effect |\n| --- | --- | --- |\n");
    for var in all() {
        table.push_str(&format!(
            "| `{}` | {} | {} |\n",
            var.name,
            var.scope.label(),
            var.purpose
        ));
    }
    table
}

#[cfg(test)]
#[path = "tests/env_vars_tests.rs"]
mod tests;
