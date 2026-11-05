//! A syscall's guest buffer that runs across two regions sitting end to
//! end reads and writes as one range, as the PPU's own loads and stores
//! do.

use super::*;
use crate::runtime::trace_bridge::MemoryView;
use cellgov_lv2::Lv2Runtime;
use cellgov_mem::{ByteRange, GuestAddr, PageSize, Region, RegionAccess};
use cellgov_ps3_abi::lv2::syscall::TTY_WRITE;

const SIZE: usize = 0x1_0000;
const EDGE: u64 = SIZE as u64;

/// A read-write region at 0 and a region of `access` right after it, as
/// two blocks a guest maps side by side.
fn adjacent(access: RegionAccess) -> GuestMemory {
    GuestMemory::from_regions(vec![
        Region::new(0, SIZE, "low", PageSize::Page64K),
        Region::with_access(EDGE, SIZE, "high", PageSize::Page64K, access),
    ])
    .expect("disjoint regions")
}

fn put(mem: &mut GuestMemory, addr: u64, bytes: &[u8]) {
    let range = ByteRange::new(GuestAddr::new(addr), bytes.len() as u64).unwrap();
    mem.apply_commit(range, bytes).expect("read-write bytes");
}

fn view<'a>(
    memory: &'a GuestMemory,
    registry: &'a crate::registry::UnitRegistry,
) -> MemoryView<'a> {
    MemoryView {
        memory,
        registry,
        current_tick: GuestTicks::ZERO,
    }
}

#[test]
fn the_lv2_view_reads_a_buffer_and_a_string_across_the_seam() {
    let mut mem = adjacent(RegionAccess::ReadWrite);
    put(&mut mem, EDGE - 3, b"abc");
    put(&mut mem, EDGE, b"def\0");
    let registry = crate::registry::UnitRegistry::new();
    let rt = view(&mem, &registry);
    assert_eq!(
        rt.read_committed(EDGE - 3, 6).as_deref(),
        Some(&b"abcdef"[..])
    );
    assert_eq!(
        rt.read_committed_until(EDGE - 3, 16, 0).as_deref(),
        Some(&b"abcdef"[..])
    );
}

#[test]
fn a_string_ending_before_a_zero_readable_neighbour_logs_no_provisional_read() {
    let mut mem = adjacent(RegionAccess::ReservedZeroReadable);
    put(&mut mem, EDGE - 3, b"ab\0");
    let registry = crate::registry::UnitRegistry::new();
    let rt = view(&mem, &registry);
    assert_eq!(
        rt.read_committed_until(EDGE - 3, 16, 0).as_deref(),
        Some(&b"ab"[..])
    );
    assert_eq!(mem.provisional_read_count(), 0);
}

#[test]
fn a_string_whose_terminator_is_a_zero_readable_byte_logs_only_that_byte() {
    let mut mem = adjacent(RegionAccess::ReservedZeroReadable);
    put(&mut mem, EDGE - 2, b"ab");
    let registry = crate::registry::UnitRegistry::new();
    let rt = view(&mem, &registry);
    assert_eq!(
        rt.read_committed_until(EDGE - 2, 16, 0).as_deref(),
        Some(&b"ab"[..])
    );
    let log = mem.drain_provisional_reads();
    assert_eq!(log.len(), 1);
    assert_eq!((log[0].addr, log[0].len, log[0].hits), (EDGE, 1, 1));
}

#[test]
fn the_lv2_view_writes_across_the_seam_only_into_read_write_bytes() {
    let registry = crate::registry::UnitRegistry::new();
    let open = adjacent(RegionAccess::ReadWrite);
    assert!(view(&open, &registry).writable(EDGE - 2, 4));
    let reserved = adjacent(RegionAccess::ReservedZeroReadable);
    assert!(!view(&reserved, &registry).writable(EDGE - 2, 4));
    assert!(view(&reserved, &registry).writable(EDGE - 4, 4));
}

#[test]
fn a_tty_write_from_a_buffer_across_the_seam_logs_every_byte() {
    let mut mem = adjacent(RegionAccess::ReadWrite);
    put(&mut mem, EDGE - 4, b"seam");
    put(&mut mem, EDGE, b"less");
    let mut rt = Runtime::new(mem, Budget::new(1), 100);
    let mut syscall_args = [0u64; 9];
    syscall_args[0] = TTY_WRITE;
    syscall_args[2] = EDGE - 4;
    syscall_args[3] = 8;
    syscall_args[4] = 0x100;
    rt.registry_mut().register_with(|id| Lv2SyscallEmitterUnit {
        id,
        steps: Cell::new(0),
        syscall_args,
    });
    let mut s = rt.step().unwrap();
    rt.commit_step(&mut s).unwrap();
    assert_eq!(rt.lv2_host().observability().tty_log, b"seamless");
}
