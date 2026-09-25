//! The thread, process and identity state keeps its partial of the
//! sync-state sum through every path that changes it.

use super::*;
use crate::dispatch::{SpuInitState, SpuLoadImage};
use crate::host::process::{ProcessEntry, ProcessTable};
use crate::host::test_support::primary_attrs;
use crate::image::{LsSegment, SpuImageHandle};
use crate::ppu_thread::{PpuThreadId, PpuThreadTable};
use crate::thread_group::GroupState;
use cellgov_event::UnitId;

/// Apply `$change`; the host partial must move and equal the rebuild,
/// in release too.
macro_rules! step {
    ($host:ident, $h:ident, $what:expr, $change:expr) => {{
        let _ = $change;
        let now = $host.sync_partial();
        assert_eq!(now, $host.sync_partial_from_scratch(), "after {}", $what);
        assert_ne!(now, $h, "{} did not move the partial", $what);
        $h = now;
        let _ = $h;
    }};
}

fn child(exit_status: Option<i32>) -> ProcessEntry {
    ProcessEntry {
        ppid: cellgov_ps3_abi::lv2::process::BOOT_PROCESS_PID,
        authority_id: 0x1070_0000_5600_0001,
        control_flags1: 0,
        exit_status,
    }
}

#[test]
fn every_thread_process_and_identity_change_moves_the_partial() {
    let mut host = Lv2Host::new();
    let mut h = host.sync_partial();
    let main = UnitId::new(0);
    step!(
        host,
        h,
        "primary seed",
        host.seed_primary_ppu_thread(main, primary_attrs())
    );
    let kid = host
        .state
        .ppu_threads
        .create(UnitId::new(1), primary_attrs());
    let kid = kid.unwrap();
    assert_ne!(
        host.sync_partial(),
        h,
        "child create did not move the partial"
    );
    h = host.sync_partial();
    step!(
        host,
        h,
        "join waiter",
        host.state
            .ppu_threads
            .add_join_waiter(kid, PpuThreadId::PRIMARY)
    );
    step!(host, h, "priority write", {
        host.state.ppu_threads.get_mut(kid).unwrap().attrs.priority = 7;
    });
    step!(
        host,
        h,
        "finish",
        host.state.ppu_threads.mark_finished(kid, 3)
    );
    step!(
        host,
        h,
        "alias",
        host.state
            .ppu_threads
            .alias_unit(UnitId::new(9), PpuThreadId::PRIMARY)
    );
    step!(
        host,
        h,
        "child stack",
        host.allocate_child_stack(0x10_000, 0x10)
    );
    let gid = host.state.groups.create(1).unwrap();
    assert_ne!(
        host.sync_partial(),
        h,
        "group create did not move the partial"
    );
    h = host.sync_partial();
    let handle = SpuImageHandle::new(5).unwrap();
    step!(
        host,
        h,
        "slot init",
        host.state
            .groups
            .initialize_thread(gid, 0, handle, [1, 2, 3, 4])
    );
    step!(host, h, "slot image", {
        host.state
            .groups
            .get_mut(gid)
            .unwrap()
            .slots
            .get_mut(&0)
            .unwrap()
            .init = Some(SpuInitState {
            image: SpuLoadImage::Elf(vec![1, 2, 3]),
            entry_pc: 0x100,
            stack_ptr: 0x3FFF0,
            args: [0; 4],
            group_id: gid,
        });
    });
    step!(host, h, "group start", {
        host.state.groups.get_mut(gid).unwrap().state = GroupState::Running;
    });
    step!(
        host,
        h,
        "record spu",
        host.state.groups.record_spu(UnitId::new(2), gid, 0)
    );
    step!(
        host,
        h,
        "spu finish",
        host.state.groups.notify_spu_finished(UnitId::new(2))
    );
    step!(
        host,
        h,
        "lwmutex hold",
        host.lwmutex_holds_inc(PpuThreadId::PRIMARY)
    );
    step!(
        host,
        h,
        "second hold",
        host.lwmutex_holds_inc(PpuThreadId::PRIMARY)
    );
    step!(
        host,
        h,
        "hold release",
        host.lwmutex_holds_dec(PpuThreadId::PRIMARY)
    );
    step!(
        host,
        h,
        "child process",
        host.state.processes.insert_child(0x0100_0501, child(None))
    );
    step!(
        host,
        h,
        "unit binding",
        host.state.processes.bind_unit(UnitId::new(1), 0x0100_0501)
    );
    step!(host, h, "boot exit", {
        host.state.processes.boot_mut().exit_status = Some(0);
    });
    step!(
        host,
        h,
        "timer count",
        host.state.process_counts.timer_inc()
    );
    step!(
        host,
        h,
        "firmware identity",
        host.set_firmware_identity("4.85", [7; 32])
    );
}

#[test]
fn an_exit_status_of_zero_differs_from_no_exit() {
    let build = |status| {
        let mut t = ProcessTable::new_boot();
        t.insert_child(0x0100_0501, child(status));
        t.sync_partial()
    };
    assert_ne!(build(None), build(Some(0)));
}

#[test]
fn every_word_of_the_pup_digest_moves_the_partial() {
    let build = |digest: [u8; 32]| {
        let mut host = Lv2Host::new();
        host.set_firmware_identity("4.85", digest);
        host.sync_partial()
    };
    let base = build([0; 32]);
    let word_set = |word: usize, byte: usize| {
        let mut digest = [0u8; 32];
        digest[word * 8 + byte] = 1;
        digest
    };
    for word in 0..4 {
        assert_ne!(build(word_set(word, 7)), base, "digest word {word}");
    }
    assert_ne!(build(word_set(0, 0)), build(word_set(1, 0)));
}

#[test]
fn join_waiters_in_another_order_hash_differently() {
    let build = |order: [u64; 2]| {
        let mut t = PpuThreadTable::new();
        t.insert_primary(UnitId::new(0), primary_attrs());
        for raw in order {
            t.add_join_waiter(PpuThreadId::PRIMARY, PpuThreadId::new(raw));
        }
        t.sync_partial()
    };
    assert_ne!(
        build([0x0100_0001, 0x0100_0002]),
        build([0x0100_0002, 0x0100_0001])
    );
}

#[test]
fn spu_image_content_and_placement_move_the_partial() {
    let build = |image: SpuLoadImage| {
        let mut host = Lv2Host::new();
        let gid = host.state.groups.create(1).unwrap();
        host.state
            .groups
            .initialize_thread(gid, 0, SpuImageHandle::new(5).unwrap(), [0; 4])
            .unwrap();
        host.state
            .groups
            .get_mut(gid)
            .unwrap()
            .slots
            .get_mut(&0)
            .unwrap()
            .init = Some(SpuInitState {
            image,
            entry_pc: 0,
            stack_ptr: 0,
            args: [0; 4],
            group_id: gid,
        });
        host.sync_partial()
    };
    let segment = |ls_start, bytes: &[u8]| LsSegment {
        ls_start,
        bytes: bytes.to_vec(),
    };
    let partials = [
        build(SpuLoadImage::Elf(vec![1, 2, 3])),
        build(SpuLoadImage::Elf(vec![1, 2, 4])),
        build(SpuLoadImage::Segments(vec![segment(0, &[1, 2, 3])])),
        build(SpuLoadImage::Segments(vec![segment(0x80, &[1, 2, 3])])),
        build(SpuLoadImage::Segments(vec![
            segment(0, &[1]),
            segment(0x80, &[2]),
        ])),
        build(SpuLoadImage::Segments(vec![
            segment(0, &[2]),
            segment(0x80, &[1]),
        ])),
    ];
    for (i, a) in partials.iter().enumerate() {
        for b in &partials[i + 1..] {
            assert_ne!(a, b);
        }
    }
}

/// Computed outside the crate from the SplitMix64 key stream of the
/// sync-state lanes.
#[test]
fn ppu_thread_partial_wire_format_golden() {
    let mut t = PpuThreadTable::new();
    t.insert_primary(UnitId::new(0), primary_attrs());
    assert_eq!(t.sync_partial(), 0x84df_5bbc_03e2_ded1_01b9_5706_dd47_c3e0);
}

/// Computed outside the crate from the SplitMix64 key stream of the
/// sync-state lanes.
#[test]
fn process_partial_wire_format_golden() {
    assert_eq!(
        ProcessTable::new_boot().sync_partial(),
        0xecd5_2fc6_3095_7ec2_92a8_c17b_9619_a593
    );
}

/// Computed outside the crate from the SplitMix64 key streams of the
/// sync-state lanes and the mixer digest.
#[test]
fn thread_group_partial_wire_format_golden() {
    let mut t = crate::thread_group::ThreadGroupTable::new();
    let gid = t.create(1).unwrap();
    t.initialize_thread(gid, 0, SpuImageHandle::new(5).unwrap(), [1, 2, 3, 4])
        .unwrap();
    t.get_mut(gid).unwrap().slots.get_mut(&0).unwrap().init = Some(SpuInitState {
        image: SpuLoadImage::Elf(vec![1, 2, 3]),
        entry_pc: 0x100,
        stack_ptr: 0x3FFF0,
        args: [0; 4],
        group_id: gid,
    });
    assert_eq!(t.sync_partial(), 0x940e_d36c_bc77_085d_3baa_68d6_5925_27f9);
}

/// The difference the counters, the lwmutex holds and the firmware
/// identity make to a fresh host's partial, computed outside the crate
/// from the SplitMix64 key stream of the sync-state lanes.
#[test]
fn counters_holds_and_firmware_wire_format_golden() {
    let mut host = Lv2Host::new();
    let fresh = host.sync_partial();
    host.state.process_counts.timer_inc();
    host.lwmutex_holds_inc(PpuThreadId::PRIMARY);
    host.lwmutex_holds_inc(PpuThreadId::PRIMARY);
    let mut digest = [0u8; 32];
    for (i, byte) in digest.iter_mut().enumerate() {
        *byte = i as u8;
    }
    host.state.firmware_identity = Some(crate::host::lv2_host::FirmwareIdentity {
        image_version_hash: 0x1234,
        pup_sha256_bytes: digest,
    });
    assert_eq!(
        host.sync_partial().wrapping_sub(fresh),
        0x5928_87d1_8806_b5b4_dbd1_030b_f64f_9529
    );
}
