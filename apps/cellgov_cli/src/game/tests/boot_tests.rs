//! Boot flag-conflict checks and the liblv2 once-mutex host-handoff witness.

use super::check_strict_reserved_vs_rsx_mirror;

#[test]
fn rejects_strict_reserved_with_rsx_mirror() {
    let err = check_strict_reserved_vs_rsx_mirror(true, true).unwrap_err();
    assert_eq!(err, super::StrictReservedConflict::RsxMirror);
    let msg = err.to_string();
    assert!(msg.contains("--strict-reserved"));
    assert!(msg.contains("rsx_mirror"));
}

#[test]
fn accepts_strict_reserved_alone() {
    assert!(check_strict_reserved_vs_rsx_mirror(true, false).is_ok());
}

#[test]
fn accepts_rsx_mirror_alone() {
    assert!(check_strict_reserved_vs_rsx_mirror(false, true).is_ok());
}

#[test]
fn accepts_neither() {
    assert!(check_strict_reserved_vs_rsx_mirror(false, false).is_ok());
}

use super::{assert_gating_state_coherent_with_host, LIBLV2_ONCE_MUTEX_SLOT};
use cellgov_core::Runtime;
use cellgov_time::Budget;

fn build_witness_test_rt() -> Runtime {
    // 0x103a49d8 sits inside the main region; 0x10500000 is
    // ample headroom past liblv2's load base.
    let mem = cellgov_mem::GuestMemory::from_regions(vec![cellgov_mem::Region::new(
        0,
        0x1050_0000,
        "main",
        cellgov_mem::PageSize::Page64K,
    )])
    .expect("witness test mem layout");
    Runtime::new(mem, Budget::new(1), 1)
}

fn stamp_mutex_id(rt: &mut Runtime, id: u32) {
    let range = cellgov_mem::ByteRange::new(cellgov_mem::GuestAddr::new(LIBLV2_ONCE_MUTEX_SLOT), 4)
        .expect("range");
    rt.memory_mut()
        .apply_commit(range, &id.to_be_bytes())
        .expect("stamp once-mutex id");
}

#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "lv2 host handoff witness")]
fn lv2_host_handoff_witness_fires_red_on_stale_id() {
    let mut rt = build_witness_test_rt();
    stamp_mutex_id(&mut rt, 0x4000_0005);
    assert_gating_state_coherent_with_host(&rt, true);
}

#[test]
fn witness_passes_when_id_is_zero() {
    let rt = build_witness_test_rt();
    assert_gating_state_coherent_with_host(&rt, true);
}

#[test]
fn witness_skipped_when_no_modules_loaded() {
    let mut rt = build_witness_test_rt();
    stamp_mutex_id(&mut rt, 0x4000_0005);
    assert_gating_state_coherent_with_host(&rt, false);
}

#[test]
fn witness_passes_when_id_lives_in_host() {
    use cellgov_lv2::sync_primitives::MutexAttrs;
    let mut rt = build_witness_test_rt();
    let id: u32 = 0x4000_0007;
    rt.lv2_host_mut()
        .mutexes_mut()
        .create_with_id(id, MutexAttrs::default())
        .expect("create witness mutex");
    stamp_mutex_id(&mut rt, id);
    assert_gating_state_coherent_with_host(&rt, true);
}

#[test]
fn cellsysutil_seed_covers_both_slots_with_v256_ring() {
    let seed = super::cellsysutil_system_seed();
    assert_eq!(
        seed.shm_ipc_key,
        cellgov_ps3_abi::system_ipc::CELLSYSUTIL_SHM_IPC_KEY
    );
    for slot_base in [0u32, 0x8000] {
        let field = |off: u32| -> &[u8] {
            &seed
                .writes
                .iter()
                .find(|(o, _)| *o == slot_base + off)
                .unwrap_or_else(|| panic!("missing seed write at slot+{off:#x}"))
                .1
        };
        assert_eq!(field(0), 0x40u32.to_be_bytes());
        assert_eq!(field(4), 256u32.to_be_bytes(), "limit");
        // The six measured dispatcher field budgets drain inside the
        // seeded ring.
        let limit = u32::from_be_bytes(field(4).try_into().unwrap());
        assert!(56 + 8 + 76 + 4 + 22 + 10 <= limit);
        assert_eq!(field(8), 0u32.to_be_bytes(), "read_pos");
        assert_eq!(field(12), 256u32.to_be_bytes(), "write_pos");
        assert_eq!(field(16), 0u32.to_be_bytes(), "cursor");
        assert_eq!(field(20), 1u32.to_be_bytes(), "state");
        assert_eq!(field(30), [0u8], "predicate");
        let payload = field(0x40);
        assert_eq!(payload.len(), 256);
        assert!(payload.iter().all(|&b| b == 0));
    }
}

#[test]
fn cellsysutil_seed_writes_stay_inside_the_64k_shm() {
    let seed = super::cellsysutil_system_seed();
    for (offset, bytes) in &seed.writes {
        assert!(
            u64::from(*offset) + bytes.len() as u64 <= 0x10000,
            "seed write at +{offset:#x} ({} bytes) exceeds the 64 KiB shm",
            bytes.len(),
        );
    }
}

// Sizing and placement rules for a spawned child image. Imported
// here so the nested modules below can reach them through `super`.
use super::{child_exit_stub_addr, decode_primary_stacksize, spawned_child_region_size};

use super::step_call_cap;

mod step_call_cap_tests {
    use super::step_call_cap;

    #[test]
    fn an_exact_multiple_reaches_every_requested_instruction() {
        // The bench-boot default: 100M instructions at the
        // architectural 256-instruction step grant.
        assert_eq!(step_call_cap(100_000_000, 256), (390_625, 100_000_000));
        assert_eq!(step_call_cap(256, 256), (1, 256));
    }

    #[test]
    fn a_remainder_is_unreachable_and_shows_in_the_effective_cap() {
        // The run-game default: 100_000 is not a multiple of 256, so
        // 160 requested instructions can never be retired.
        assert_eq!(step_call_cap(100_000, 256), (390, 99_840));
        // Just under two grants still buys only one.
        assert_eq!(step_call_cap(511, 256), (1, 256));
    }

    #[test]
    fn a_budget_of_one_passes_the_instruction_cap_through() {
        assert_eq!(step_call_cap(7, 1), (7, 7));
        assert_eq!(step_call_cap(0, 1), (0, 0));
    }
}

mod child_exit_stub_addr_tests {
    use super::{child_exit_stub_addr, spawned_child_region_size};

    /// ELF64 big-endian header plus one PT_LOAD per
    /// `(p_vaddr, p_filesz, p_memsz)`. Only the headers matter --
    /// `required_memory_size` never reads segment contents.
    fn elf64_be_with_loads(segments: &[(u64, u64, u64)]) -> Vec<u8> {
        const EHDR: usize = 64;
        const PHENT: usize = 56;
        const PT_LOAD: u32 = 1;
        let mut out = vec![0u8; EHDR + PHENT * segments.len()];
        out[..4].copy_from_slice(&[0x7F, b'E', b'L', b'F']);
        out[4] = 2; // ELFCLASS64
        out[5] = 2; // ELFDATA2MSB
        out[6] = 1; // EV_CURRENT
        out[32..40].copy_from_slice(&(EHDR as u64).to_be_bytes()); // e_phoff
        out[54..56].copy_from_slice(&(PHENT as u16).to_be_bytes()); // e_phentsize
        out[56..58].copy_from_slice(&(segments.len() as u16).to_be_bytes()); // e_phnum
        for (i, &(vaddr, filesz, memsz)) in segments.iter().enumerate() {
            let b = EHDR + PHENT * i;
            out[b..b + 4].copy_from_slice(&PT_LOAD.to_be_bytes());
            out[b + 16..b + 24].copy_from_slice(&vaddr.to_be_bytes());
            out[b + 32..b + 40].copy_from_slice(&filesz.to_be_bytes());
            out[b + 40..b + 48].copy_from_slice(&memsz.to_be_bytes());
            // A large p_align is what would tempt a reader to think the
            // image occupies more than its highest segment end.
            out[b + 48..b + 56].copy_from_slice(&0x1_0000u64.to_be_bytes());
        }
        out
    }

    #[test]
    fn the_stub_never_lands_on_a_segment_byte() {
        // Sparse, out-of-order, memsz > filesz, a zero-memsz segment,
        // and a highest segment that ends unaligned against its own
        // p_align -- every shape that could put a written byte above
        // the reported image end.
        let layouts: &[&[(u64, u64, u64)]] = &[
            &[(0, 0x100, 0x1000)],
            &[(0, 1, 0x11)],
            &[
                (0x1_0000, 0x100, 0x1000),
                (0x1000_0000, 0x10, 0x8000),
                (0x2_0000, 0x40, 0x40),
            ],
            &[(0x1000_0000, 0x10, 0x8001), (0, 0, 0)],
            &[(0x8000, 0, 0x4000)],
        ];
        for segments in layouts {
            let elf = elf64_be_with_loads(segments);
            let required = cellgov_ppu::loader::required_memory_size(&elf)
                .expect("synthetic ELF headers parse");
            let stub = child_exit_stub_addr(required);
            for &(vaddr, _, memsz) in segments.iter() {
                if memsz == 0 {
                    continue;
                }
                assert!(
                    stub >= vaddr + memsz || stub + 8 <= vaddr,
                    "stub 0x{stub:x}+8 overlaps segment 0x{vaddr:x}+0x{memsz:x}",
                );
            }
        }
    }

    #[test]
    fn the_stub_is_instruction_aligned() {
        for required in [0, 1, 8, 0x11, 0x1001_0010] {
            assert_eq!(child_exit_stub_addr(required) % 16, 0);
        }
    }

    #[test]
    fn an_image_with_no_loadable_bytes_keeps_address_zero_reserved() {
        assert_eq!(child_exit_stub_addr(0), 16);
    }

    #[test]
    fn the_stub_fits_inside_the_region_the_caller_sizes() {
        // Pairing with the sizing rule is the contract that keeps the
        // stub write in bounds and below the initial SP. The last
        // entry is the 4 GiB EA ceiling `required_memory_size` caps
        // every segment end at, so nothing larger can reach here.
        for required in [
            0,
            0x1001_0010,
            0x1001_b000,
            0x1001_b001,
            0x1002_0001,
            0x2000_0000,
            0x1_0000_0000,
        ] {
            let size = spawned_child_region_size(required).expect("sizable child");
            let stub_end = child_exit_stub_addr(required) + 8;
            let stack_top = size as u64 - 0x1000;
            assert!(
                stub_end <= stack_top,
                "stub for required=0x{required:x} must end at or below the SP",
            );
            assert!(
                stub_end <= size as u64,
                "stub for required=0x{required:x} must end inside the region",
            );
        }
    }
}

mod spawned_child_region_size_tests {
    use super::spawned_child_region_size;

    #[test]
    fn a_psl1ght_shaped_child_keeps_the_microtest_floor() {
        // The committed process_spawn_wait child: PT_LOADs end at
        // 0x1001_0010, well under the floor minus the SP pad and the
        // minimum stack.
        assert_eq!(spawned_child_region_size(0x1001_0010), Ok(0x1002_0000));
        assert_eq!(spawned_child_region_size(0), Ok(0x1002_0000));
        // Boundary: image end exactly MIN_CHILD_STACK below the SP.
        assert_eq!(spawned_child_region_size(0x1001_b000), Ok(0x1002_0000));
    }

    #[test]
    fn a_child_larger_than_the_floor_grows_instead_of_failing() {
        // 64K-aligned required plus 128K headroom.
        assert_eq!(spawned_child_region_size(0x1002_0001), Ok(0x1005_0000));
        assert_eq!(spawned_child_region_size(0x2000_0000), Ok(0x2002_0000));
    }

    #[test]
    fn a_child_that_would_squeeze_the_stack_grows_instead_of_colliding() {
        // One byte past the floor's minimum-stack boundary: keeping
        // the floor would leave the SP less than 0x4000 above the
        // image end, so the sizing grows the region instead.
        assert_eq!(spawned_child_region_size(0x1001_b001), Ok(0x1004_0000));
        // The old boundary shape: image end flush against the SP
        // (zero stack depth under the pre-fix floor rule).
        assert_eq!(spawned_child_region_size(0x1001_f000), Ok(0x1004_0000));
    }

    #[test]
    fn a_required_size_near_usize_max_reports_overflow_not_panic() {
        let err = spawned_child_region_size(usize::MAX - 0x10).unwrap_err();
        assert!(
            matches!(
                &err,
                cellgov_core::ProcessSpawnLoadError::RegionSize { detail }
                    if detail.contains("overflows")
            ),
            "got: {err}"
        );
    }
}

mod primary_stacksize_tests {
    use super::decode_primary_stacksize;

    #[test]
    fn a_sentinel_stacksize_declaration_decodes_to_its_byte_count() {
        for (sentinel, bytes) in [
            (0x10, 32 * 1024),
            (0x20, 64 * 1024),
            (0x30, 96 * 1024),
            (0x40, 128 * 1024),
            (0x50, 256 * 1024),
            (0x60, 512 * 1024),
            (0x70, 1024 * 1024),
        ] {
            assert_eq!(decode_primary_stacksize(sentinel), bytes);
        }
    }

    #[test]
    fn a_raw_byte_count_declaration_passes_through_unchanged() {
        assert_eq!(decode_primary_stacksize(0), 0);
        assert_eq!(decode_primary_stacksize(0x9000), 0x9000);
        assert_eq!(decode_primary_stacksize(0x40000), 0x40000);
        assert_eq!(decode_primary_stacksize(0x100000), 0x100000);
    }
}
