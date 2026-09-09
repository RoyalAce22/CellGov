//! Which child statuses read as an interrupt, on each platform.

use super::*;

#[cfg(unix)]
mod unix {
    use super::*;
    use std::os::unix::process::ExitStatusExt as _;

    /// A wait status: the low seven bits carry the signal that killed
    /// the process, and an exit code sits in the byte above them.
    fn killed_by(signal: i32) -> ExitStatus {
        ExitStatus::from_raw(signal)
    }

    fn exited(code: i32) -> ExitStatus {
        ExitStatus::from_raw(code << 8)
    }

    #[test]
    fn a_child_killed_by_sigint_was_interrupted() {
        assert!(was_interrupted(killed_by(signal_hook::consts::SIGINT)));
    }

    #[test]
    fn a_child_that_exited_130_chose_that_status_and_was_not_interrupted() {
        assert!(!was_interrupted(exited(130)));
        assert!(!was_interrupted(exited(0)));
    }

    #[test]
    fn a_child_killed_by_another_signal_was_not_interrupted() {
        assert!(!was_interrupted(killed_by(signal_hook::consts::SIGTERM)));
        assert!(!was_interrupted(killed_by(signal_hook::consts::SIGKILL)));
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::os::windows::process::ExitStatusExt as _;

    #[test]
    fn a_child_that_exited_with_the_control_c_status_was_interrupted() {
        assert!(was_interrupted(ExitStatus::from_raw(CONTROL_C_EXIT)));
    }

    #[test]
    fn any_other_status_was_not() {
        assert!(!was_interrupted(ExitStatus::from_raw(0)));
        assert!(!was_interrupted(ExitStatus::from_raw(130)));
        // STATUS_ACCESS_VIOLATION: a crash.
        assert!(!was_interrupted(ExitStatus::from_raw(0xC000_0005)));
    }
}
