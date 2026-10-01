//! The exit-code contract: one code per failure class, no two classes
//! sharing one, and every error variant mapped.

use strum::VariantArray;

use super::*;

#[test]
fn every_class_has_its_own_code_and_ok_is_zero() {
    let codes: Vec<i32> = ExitCode::VARIANTS.iter().map(|c| c.code()).collect();
    assert_eq!(codes, [0, 1, 2, 3, 4, 5, 6]);
    assert_eq!(ExitCode::Ok.code(), 0);
}

#[test]
fn each_error_maps_to_the_class_its_message_names() {
    let cases: Vec<(RunnerPs3Error, ExitCode)> = vec![
        (
            RunnerPs3Error::Usage("no --host".to_string()),
            ExitCode::Usage,
        ),
        (
            RunnerPs3Error::LocalIo {
                path: PathBuf::from("manifest.toml"),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            },
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
            RunnerPs3Error::Transport("FTP 530".to_string()),
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
