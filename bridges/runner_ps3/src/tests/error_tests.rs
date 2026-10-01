//! The exit-code contract: one code per failure class, no two classes
//! sharing one, and every error variant mapped.

use cellgov_observation::console_profile::LoadLimits;
use cellgov_observation::hardware_capture::LoadReading;
use strum::VariantArray;

use super::*;
use crate::load::Thermal;

#[test]
fn every_class_has_its_own_code_and_ok_is_zero() {
    let codes: Vec<i32> = ExitCode::VARIANTS.iter().map(|c| c.code()).collect();
    assert_eq!(codes, [0, 1, 2, 3, 4, 5, 6, 7]);
    assert_eq!(ExitCode::Ok.code(), 0);
}

#[test]
fn each_error_maps_to_the_class_its_message_names() {
    let hot = LoadReading {
        cpu_c: 85,
        rsx_c: 70,
        fan_percent: Some(60),
    };
    let limits = LoadLimits {
        hot_c: 80,
        cool_c: 72,
        hdd_floor_mib: 1024,
        wait_poll_s: 15,
        wait_limit_s: 1800,
    };
    let cases: Vec<(RunnerPs3Error, ExitCode)> = vec![
        (
            RunnerPs3Error::Usage("no --host".to_string()),
            ExitCode::Usage,
        ),
        (
            RunnerPs3Error::LocalRead {
                path: PathBuf::from("manifest.toml"),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            },
            ExitCode::Usage,
        ),
        (
            RunnerPs3Error::LocalWrite {
                path: PathBuf::from("tests/micro/x/ps3/cech20-cex-493"),
                source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            },
            ExitCode::Local,
        ),
        (RunnerPs3Error::HostClock, ExitCode::Local),
        (
            RunnerPs3Error::from(
                cellgov_observation::manifest::parse_console("[test]").expect_err("no observe"),
            ),
            ExitCode::Usage,
        ),
        (
            RunnerPs3Error::Refused {
                reason: "another runner holds the console".to_string(),
                clear_with: "runner_ps3 unlock --host 10.77.0.2".to_string(),
            },
            ExitCode::Refused,
        ),
        (
            RunnerPs3Error::from(LeaseError::Held {
                path: PathBuf::from("cellgov_runner_ps3_10.77.0.2.lease"),
                host: "10.77.0.2".to_string(),
                holder: "pid=1 holder=x".to_string(),
                unlock_with: "runner_ps3 unlock --host 10.77.0.2".to_string(),
            }),
            ExitCode::Refused,
        ),
        (
            RunnerPs3Error::from(LeaseError::Io {
                path: PathBuf::from("cellgov_runner_ps3_10.77.0.2.lease"),
                source: std::io::Error::from(std::io::ErrorKind::PermissionDenied),
            }),
            ExitCode::Local,
        ),
        (
            RunnerPs3Error::from(ConsoleError::OperatorMissing {
                field: "model",
                flag: "--model",
            }),
            ExitCode::Usage,
        ),
        (
            RunnerPs3Error::from(ConsoleError::PageMissing { field: "firmware" }),
            ExitCode::Transport,
        ),
        (
            RunnerPs3Error::from(ConsoleError::Contradiction {
                field: "model",
                page: "CECH-2501A".to_string(),
                operator: "CECH-2001A".to_string(),
            }),
            ExitCode::Refused,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::Mismatch {
                profile: "cech20-cex-493".to_string(),
                mismatches: Vec::new(),
                satisfied: Vec::new(),
            }),
            ExitCode::Refused,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::UnknownProfile {
                claimed: "cech25".to_string(),
                known: "cech20-cex-493".to_string(),
            }),
            ExitCode::Refused,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::UnknownReference("cech25".to_string())),
            ExitCode::Usage,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::NoModels("cech25".to_string())),
            ExitCode::Usage,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::Io {
                path: PathBuf::from("console_profiles.toml"),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            }),
            ExitCode::Usage,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::from(
                toml::from_str::<toml::Table>("=").expect_err("bad toml"),
            )),
            ExitCode::Usage,
        ),
        (
            RunnerPs3Error::from(ConsoleProfileError::LoadLimits("cool_c 80".to_string())),
            ExitCode::Usage,
        ),
        (
            RunnerPs3Error::from(LoadError::Hot {
                thermal: Thermal::Hot,
                reading: hot,
                limits,
                poll_with: "runner_ps3 status --host 10.77.0.2".to_string(),
            }),
            ExitCode::Refused,
        ),
        (
            RunnerPs3Error::from(LoadError::StillHot {
                thermal: Thermal::Cooling,
                reading: hot,
                limits,
                waited_s: 1800,
            }),
            ExitCode::Refused,
        ),
        (
            RunnerPs3Error::from(LoadError::Full {
                free_bytes: 1,
                required_bytes: 2,
            }),
            ExitCode::Refused,
        ),
        (
            RunnerPs3Error::from(LoadError::Unstated("CPU temperature")),
            ExitCode::Transport,
        ),
        (
            RunnerPs3Error::from(crate::transport::TransportError::ReplyTruncated),
            ExitCode::Transport,
        ),
        (
            RunnerPs3Error::Timeout {
                result_path: "/dev_hdd0/tmp/cgov_spu_fixed_value.bin".to_string(),
                timeout_ms: 30_000,
            },
            ExitCode::Timeout,
        ),
        (
            RunnerPs3Error::Frame("magic absent".to_string()),
            ExitCode::Frame,
        ),
        (
            RunnerPs3Error::Cleanup {
                remaining: "/dev_hdd0/game/CGOV00001".to_string(),
            },
            ExitCode::Cleanup,
        ),
    ];
    for (error, code) in cases {
        assert_eq!(error.exit_code(), code, "{error}");
    }
}

#[test]
fn a_refusal_prints_the_command_that_clears_it() {
    let error = RunnerPs3Error::Refused {
        reason: "a result file already sits at /dev_hdd0/tmp/cgov_x.bin".to_string(),
        clear_with: "runner_ps3 cleanup --host 10.77.0.2 --manifest tests/micro/x/manifest.toml"
            .to_string(),
    };
    assert_eq!(
        error.to_string(),
        "refused: a result file already sits at /dev_hdd0/tmp/cgov_x.bin; clear it with \
         `runner_ps3 cleanup --host 10.77.0.2 --manifest tests/micro/x/manifest.toml`"
    );
}
