//! The interlock: the load read from the status page in both its
//! encodings, the hysteresis between the ceiling and the floor, the
//! marker that carries it between readings, and `admit`'s refusals and
//! wait.

use cellgov_testkit::scratch::ScratchDir;

use super::*;
use crate::memory_console::MemoryConsole;
use crate::ExitCode;

/// webMAN's status page as the console served it, with every
/// identifier replaced by a synthetic value of the same shape.
const PAGE: &str = include_str!("fixtures/cpursx.html");

const HOST: &str = "10.77.0.2";

const LIMITS: LoadLimits = LoadLimits {
    hot_c: 80,
    cool_c: 72,
    hdd_floor_mib: 1024,
    wait_poll_s: 15,
    wait_limit_s: 60,
};

/// A page in the shape the live console serves: the sign itself, the
/// Fahrenheit readings before the Celsius ones, a fan line and a USB
/// device's free space.
fn live_page(cpu_c: u32, rsx_c: u32, hdd: &str) -> String {
    format!(
        "<a class=\"s\" href=\"/cpursx.ps3?dn\">CPU: 152\u{b0}F (MAX: 154\u{b0}F)<br>RSX: \
         147\u{b0}F</a><a class=\"s\" href=\"/cpursx.ps3?up\">CPU: {cpu_c}\u{b0}C (MAX: \
         {cpu_c}\u{b0}C)<br>RSX: {rsx_c}\u{b0}C</a><a href=\"/dev_usb000\">USB000:  28.8 GB \
         free</a><a href=\"/dev_hdd0\">HDD:  {hdd} free</a><a class=\"s\" \
         href=\"/cpursx.ps3?mode\">FAN SPEED:  40% (0x66)</a>"
    )
}

#[test]
fn a_host_folds_to_a_marker_file_name() {
    let dir = Path::new("markers");
    assert_eq!(
        hot_path(dir, "10.77.0.2"),
        dir.join("cellgov_runner_ps3_10.77.0.2.hot")
    );
    assert_eq!(
        hot_path(dir, "ps3:80/../x"),
        dir.join("cellgov_runner_ps3_ps3_80_.._x.hot")
    );
}

#[test]
fn the_load_reads_from_the_fixture_page_and_the_live_shape() {
    assert_eq!(
        parse_load(PAGE),
        ConsoleLoad {
            cpu_c: Some(67),
            rsx_c: Some(64),
            fan_percent: None,
            hdd_free_bytes: Some(97 * (1 << 30) + (1 << 29)),
        }
    );
    assert_eq!(
        parse_load(&live_page(68, 64, "97.5 GB")),
        ConsoleLoad {
            cpu_c: Some(68),
            rsx_c: Some(64),
            fan_percent: Some(40),
            hdd_free_bytes: Some(97 * (1 << 30) + (1 << 29)),
        }
    );
    assert_eq!(parse_load("<html></html>"), ConsoleLoad::default());
}

#[test]
fn free_space_reads_each_unit_and_refuses_any_other() {
    assert_eq!(bytes("512", "MB"), Some(512 << 20));
    assert_eq!(bytes("1.25", "GB"), Some((1 << 30) + (1 << 28)));
    assert_eq!(bytes("3", "KB"), Some(3 << 10));
    assert_eq!(bytes("2", "TB"), Some(2 << 40));
    assert_eq!(bytes("2", "GiB"), None);
    assert_eq!(bytes("two", "GB"), None);
}

#[test]
fn between_the_floor_and_the_ceiling_the_state_before_decides() {
    for (hottest, was_hot, state) in [
        (79, false, Thermal::Cool),
        (80, false, Thermal::Hot),
        (80, true, Thermal::Hot),
        (75, true, Thermal::Cooling),
        (75, false, Thermal::Cool),
        (72, true, Thermal::Cooling),
        (71, true, Thermal::Cool),
    ] {
        assert_eq!(
            Thermal::of(hottest, &LIMITS, was_hot),
            state,
            "{hottest} C, hot before: {was_hot}"
        );
    }
}

fn reading(cpu_c: u32, rsx_c: u32) -> LoadReading {
    LoadReading {
        cpu_c,
        rsx_c,
        fan_percent: None,
    }
}

#[test]
fn the_marker_carries_a_hot_reading_until_the_console_reads_below_the_floor() {
    let scratch = cellgov_testkit::scratch::scratch();
    let marker = hot_path(&scratch, HOST);
    let assess_at = |cpu_c, rsx_c| assess(reading(cpu_c, rsx_c), &LIMITS, &scratch, HOST);
    assert_eq!(assess_at(75, 60).expect("assess"), Thermal::Cool);
    assert!(!marker.exists());
    assert_eq!(
        assess_at(70, 81).expect("assess"),
        Thermal::Hot,
        "the RSX counts"
    );
    assert!(marker.exists());
    assert_eq!(assess_at(75, 60).expect("assess"), Thermal::Cooling);
    assert!(marker.exists(), "cooling keeps the marker");
    assert_eq!(assess_at(71, 60).expect("assess"), Thermal::Cool);
    assert!(!marker.exists(), "cool removes it");
    assert_eq!(assess_at(75, 60).expect("assess"), Thermal::Cool);
}

/// A console serving `first`, then each of `later` on each further read.
fn console_serving(first: &str, later: &[String]) -> MemoryConsole {
    let mut console = MemoryConsole::empty();
    console
        .files
        .insert(STATUS_PATH.to_string(), first.as_bytes().to_vec());
    console.later_status = later.iter().map(|page| page.as_bytes().to_vec()).collect();
    console
}

fn interlock(scratch: &ScratchDir, wait_cool: bool, needs_space: bool) -> Interlock<'_> {
    Interlock {
        limits: LIMITS,
        marker_dir: scratch,
        host: HOST,
        wait_cool,
        needs_space,
        poll_with: "runner_ps3 status --host 10.77.0.2".to_string(),
    }
}

/// `admit` on a console whose first page is `first`, counting sleeps.
fn admit_on(
    console: &mut MemoryConsole,
    first: &str,
    interlock: &Interlock<'_>,
) -> (Result<LoadReading, RunnerPs3Error>, Vec<Duration>) {
    let mut slept = Vec::new();
    let result = admit(
        console,
        first,
        interlock,
        &mut |d| slept.push(d),
        &mut Transcript::new(),
    );
    (result, slept)
}

#[test]
fn a_hot_console_is_refused_with_both_temperatures_the_limits_and_the_poll() {
    let scratch = cellgov_testkit::scratch::scratch();
    let page = live_page(85, 66, "97.5 GB");
    let mut console = console_serving(&page, &[]);
    let (result, slept) = admit_on(&mut console, &page, &interlock(&scratch, false, false));
    let error = result.expect_err("hot");
    assert_eq!(error.exit_code(), ExitCode::Refused);
    assert_eq!(
        error.to_string(),
        "load: the console is hot: Cell 85 C, RSX 66 C; it is hot at 80 C and cool again only \
         below 72 C. Poll `runner_ps3 status --host 10.77.0.2` until it reads cool, or rerun \
         with --wait-cool"
    );
    assert!(slept.is_empty());
    assert!(
        console.calls.is_empty(),
        "the refusal reads nothing more: {:?}",
        console.calls
    );
}

#[test]
fn wait_cool_reads_again_until_the_console_is_below_the_floor() {
    let scratch = cellgov_testkit::scratch::scratch();
    let first = live_page(84, 70, "97.5 GB");
    let later = [live_page(76, 70, "97.5 GB"), live_page(70, 66, "97.5 GB")];
    let mut console = console_serving(&later[0], &later[1..]);
    let (result, slept) = admit_on(&mut console, &first, &interlock(&scratch, true, true));
    assert_eq!(result.expect("cooled"), {
        let mut cool = reading(70, 66);
        cool.fan_percent = Some(40);
        cool
    });
    assert_eq!(slept, [Duration::from_secs(15); 2]);
    assert_eq!(console.calls, ["FETCH /cpursx.ps3", "FETCH /cpursx.ps3"]);
    assert!(!hot_path(&scratch, HOST).exists());
}

#[test]
fn wait_cool_refuses_once_its_limit_passes() {
    let scratch = cellgov_testkit::scratch::scratch();
    let hot = live_page(75, 82, "97.5 GB");
    let cooling = live_page(75, 74, "97.5 GB");
    let later = vec![cooling.clone(); 8];
    let mut console = console_serving(&cooling, &later);
    let (result, slept) = admit_on(&mut console, &hot, &interlock(&scratch, true, false));
    let error = result.expect_err("still cooling");
    assert_eq!(error.exit_code(), ExitCode::Refused);
    assert!(
        error
            .to_string()
            .starts_with("load: the console is still cooling after 60 s of --wait-cool"),
        "{error}"
    );
    assert_eq!(slept.len(), 4, "60 s at 15 s a read");
}

#[test]
fn a_deploy_needs_the_floor_free_on_dev_hdd0() {
    let scratch = cellgov_testkit::scratch::scratch();
    let page = live_page(60, 60, "1023 MB");
    let mut console = console_serving(&page, &[]);
    let (result, _) = admit_on(&mut console, &page, &interlock(&scratch, false, true));
    let error = result.expect_err("full");
    assert_eq!(error.exit_code(), ExitCode::Refused);
    assert_eq!(
        error.to_string(),
        "load: /dev_hdd0 holds 1072693248 bytes free and a deploy needs 1073741824; free space \
         on the console"
    );
    let (result, _) = admit_on(&mut console, &page, &interlock(&scratch, false, false));
    result.expect("a verb that does not deploy needs no space");
    let (result, _) = admit_on(
        &mut console,
        &live_page(60, 60, "1 GB"),
        &interlock(&scratch, false, true),
    );
    result.expect("the floor itself is enough");
}

#[test]
fn a_page_without_a_reading_is_refused_as_a_transport_failure() {
    let scratch = cellgov_testkit::scratch::scratch();
    let mut console = MemoryConsole::empty();
    let no_rsx = PAGE.replace("RSX:", "GPU:");
    let (result, _) = admit_on(&mut console, &no_rsx, &interlock(&scratch, false, false));
    let error = result.expect_err("no RSX reading");
    assert_eq!(error.exit_code(), ExitCode::Transport);
    assert!(
        error
            .to_string()
            .contains("does not state the RSX temperature"),
        "{error}"
    );
    let no_hdd = PAGE.replace("HDD:", "HD:");
    let (result, _) = admit_on(&mut console, &no_hdd, &interlock(&scratch, false, true));
    assert!(result
        .expect_err("no free space")
        .to_string()
        .contains("does not state the free space on /dev_hdd0"),);
}
