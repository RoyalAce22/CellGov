use super::*;
use crate::group::spe::LogicalSpe;
use cellgov_event::UnitId;
use cellgov_ps3_abi::hw::ppu::CELL_PPU_TIMEBASE_HZ;
use cellgov_ps3_abi::lv1::event_port::game_os_port_id;
use cellgov_ps3_abi::lv1::spe::{game_os_spe_call, GameOsSpeCall};
use cellgov_time::GuestTicks;

/// The 4.93 kernel's boot SPE construct
/// [OFW-4.93 lv2_kernel a:0x800000000004f188], with the ids that the
/// capture `lv2_logical_spe_object` reads.
fn boot_spe() -> &'static GameOsSpeCall {
    game_os_spe_call("4.93", 0x8000_0000_0004_f188).expect("the boot SPE's ids")
}

fn boot_spe_id() -> u64 {
    boot_spe().id
}

/// The limit of the kernel's boot call when `sys.flash.ext` has bit 1
/// set: 120 times the timebase frequency
/// [OFW-4.93 lv2_kernel a:0x800000000004f260-0x800000000004f26c].
const BOOT_LIMIT: u64 = 120 * CELL_PPU_TIMEBASE_HZ;

/// The kernel's call with `spe_id` and `limit` in r3 and r4, where the
/// 4.93 kernel's boot call sets them
/// [OFW-4.93 lv2_kernel a:0x800000000004f270-0x800000000004f278].
fn request(spe_id: u64, limit: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(1),
        number: LOGICAL_SPU_SET_RUN_TIMEOUT,
        inputs: [spe_id, limit, 0, 0, 0, 0, 0, 0],
        pc: 0x8000_0000_0004_f278,
        lr: 0,
        thread: Some(0),
        problem_state: false,
        tick: GuestTicks::new(63_190),
    }
}

fn spe(id: u64, spe_type: u64) -> LogicalSpe {
    LogicalSpe {
        id,
        spe_type,
        vas_id: 2,
        area: 0x0000_4c00_0028_0000,
        shadow: 0x0000_3000_0000_6000,
        shadow_real: 0x1100_0000,
    }
}

/// A state that holds the type 1 SPE `id`.
fn spes(id: u64) -> LogicalSpeState {
    let mut spes = LogicalSpeState::default();
    spes.construct(spe(id, 1));
    spes
}

fn refused(r: &HypercallRequest, what: &'static str) -> HypercallDisposition {
    HypercallDisposition::Refused(HypercallRefusal::UnmodelledInput {
        number: LOGICAL_SPU_SET_RUN_TIMEOUT,
        unit: r.unit,
        pc: r.pc,
        what,
    })
}

fn kept(spe: u64, limit: u64) -> HypercallDisposition {
    let mut answer = HypercallAnswer::lv1_status(0, []);
    answer.push_change(SpeChange::RunTimeoutSet(RunTimeout { spe, limit }));
    HypercallDisposition::Answered(answer)
}

#[test]
fn the_boot_call_keeps_the_limit_with_r3_to_r10_cleared() {
    let d = set_run_timeout(
        &request(boot_spe_id(), BOOT_LIMIT),
        Some("4.93"),
        &spes(boot_spe_id()),
    );
    let HypercallDisposition::Answered(answer) = &d else {
        panic!("{d:?}");
    };
    assert_eq!(answer.r3, 0);
    assert_eq!(answer.outputs(), Some([0; 7]));
    assert_eq!(d, kept(boot_spe_id(), BOOT_LIMIT));
}

/// lv1's method stores r4 with no check
/// [OFW-4.93 lv1 a:0x3156B0-0x3156B8].
#[test]
fn each_limit_is_kept_as_r4_gives_it() {
    let limits = [0, 1, BOOT_LIMIT, i64::MAX as u64, 1 << 63, u64::MAX];
    for limit in limits {
        assert_eq!(
            set_run_timeout(
                &request(boot_spe_id(), limit),
                Some("4.93"),
                &spes(boot_spe_id())
            ),
            kept(boot_spe_id(), limit),
            "limit {limit:#x}"
        );
    }
}

/// lv1's handler gives only r3 and r4 to the worker
/// [OFW-4.93 lv1 a:0x31BDDC-0x31BDE4].
#[test]
fn r5_to_r10_do_not_change_the_answer() {
    let mut r = request(boot_spe_id(), BOOT_LIMIT);
    r.inputs[2..].copy_from_slice(&[u64::MAX, 1, 2, 3, 4, 0x8000_0000_0000_0000]);
    assert_eq!(
        set_run_timeout(&r, Some("4.93"), &spes(boot_spe_id())),
        kept(boot_spe_id(), BOOT_LIMIT)
    );
}

/// The boot SPE's outlet and port ids and the id 2 name objects of other
/// classes, and 12 names an object that no read gives.
#[test]
fn an_spe_id_that_names_no_constructed_spe_is_refused() {
    let what = "an SPE run timeout call on an spe id that the hypervisor did not construct";
    let port = game_os_port_id("4.93", 0, 0x8000_0000_0004_f218).expect("the boot SPE's port id");
    let outlet_0 = boot_spe().outlets[0];
    for id in [boot_spe_id() + 1, outlet_0, port, 2, 0, u64::MAX] {
        let r = request(id, BOOT_LIMIT);
        assert_eq!(
            set_run_timeout(&r, Some("4.93"), &spes(boot_spe_id())),
            refused(&r, what),
            "spe id {id}"
        );
    }
}

#[test]
fn an_spe_that_is_not_type_1_is_refused() {
    let what = "an SPE run timeout call on an SPE whose type's +192 doubleword no read gives";
    for spe_type in [0, 2] {
        let mut held = LogicalSpeState::default();
        held.construct(spe(boot_spe_id(), spe_type));
        let r = request(boot_spe_id(), BOOT_LIMIT);
        assert_eq!(
            set_run_timeout(&r, Some("4.93"), &held),
            refused(&r, what),
            "type {spe_type}"
        );
    }
}

#[test]
fn a_firmware_whose_handler_is_not_read_is_refused() {
    let what = "an SPE run timeout call at a firmware whose lv1 handler is not read";
    let r = request(boot_spe_id(), BOOT_LIMIT);
    for firmware in [None, Some("4.92"), Some("3.55")] {
        assert_eq!(
            set_run_timeout(&r, firmware, &spes(boot_spe_id())),
            refused(&r, what),
            "{firmware:?}"
        );
    }
}
