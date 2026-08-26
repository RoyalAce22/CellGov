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

use super::{
    child_exit_stub_addr, decode_primary_stacksize, primary_entry_sp, primary_stack_base_for,
    resolve_primary_prio, spawned_child_region_size, DEFAULT_PRIMARY_PRIO,
};

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
            // A large p_align exercises the case where alignment could
            // inflate the reported image end past the highest segment.
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
        // entry is the largest image the sizing accepts under the RSX
        // iomap cap.
        for required in [
            0,
            0x1001_0010,
            0x1001_b000,
            0x1001_b001,
            0x1002_0001,
            0x2000_0000,
            0x3fd0_0000,
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
    fn a_child_below_the_floor_gets_the_boot_sized_region() {
        // The committed process_spawn_wait child (PT_LOADs end at
        // 0x1001_0010), an empty image, and a game-sized image all
        // take the 1 GiB floor, so TLS_BASE and the fixed layout above
        // it are inside the region.
        assert_eq!(spawned_child_region_size(0x1001_0010), Ok(0x4000_0000));
        assert_eq!(spawned_child_region_size(0), Ok(0x4000_0000));
        assert_eq!(spawned_child_region_size(0x2000_0000), Ok(0x4000_0000));
        // Aligned image plus 2 MiB headroom exactly at the floor.
        assert_eq!(spawned_child_region_size(0x3fe0_0000), Ok(0x4000_0000));
    }

    #[test]
    fn a_child_whose_image_reaches_the_rsx_iomap_window_is_refused() {
        // One byte over: 64K alignment plus headroom crosses the cap.
        let err = spawned_child_region_size(0x3fe0_0001).unwrap_err();
        assert!(
            matches!(
                &err,
                cellgov_core::ProcessSpawnLoadError::RegionSize { detail }
                    if detail.contains("PS3_RSX_IOMAP_BASE")
            ),
            "got: {err}"
        );
        assert!(spawned_child_region_size(0x1_0000_0000).is_err());
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

mod spawned_child_code_floor_tests {
    use super::super::{child_exit_stub_addr, spawned_child_code_floor, spawned_child_region_size};

    #[test]
    fn the_code_floor_clears_the_exit_stub_at_every_alignment() {
        // A page-aligned image end (the shape `p_align` gives real
        // SELFs) puts the stub at `required` itself; the 64K-aligned
        // end is the firmware-set placement boundary; the near-boundary
        // values sit in the last 16 bytes before a page.
        for required in [
            0,
            1,
            0xFF9,
            0xFFF9,
            0x1000,
            0x1001_0000,
            0x1001_0010,
            0x1001_0ff8,
            0x1002_0000,
            0x1002_fff0,
            0x3fe0_0000,
        ] {
            let stub_end = child_exit_stub_addr(required) + 8;
            let floor = spawned_child_code_floor(required);
            assert!(
                floor >= stub_end,
                "required=0x{required:x}: code floor 0x{floor:x} sits on the stub ending \
                 at 0x{stub_end:x}",
            );
            assert_eq!(
                floor % 0x1000,
                0,
                "required=0x{required:x}: floor is page-aligned"
            );
            let size = spawned_child_region_size(required).expect("sizable child");
            assert!(
                floor < size as u64,
                "required=0x{required:x}: floor 0x{floor:x} inside the 0x{size:x} region",
            );
        }
    }

    #[test]
    fn a_page_aligned_image_end_moves_the_floor_one_page_up() {
        // The stub occupies [0x1001_0000, 0x1001_0008); a floor rounded
        // from `required` alone would be 0x1001_0000.
        assert_eq!(spawned_child_code_floor(0x1001_0000), 0x1001_1000);
        // An unaligned end keeps the floor at the next page.
        assert_eq!(spawned_child_code_floor(0x1001_0010), 0x1001_1000);
    }
}

mod primary_prio_tests {
    use super::{resolve_primary_prio, DEFAULT_PRIMARY_PRIO};

    #[test]
    fn an_absent_param_segment_takes_the_kernel_default() {
        assert_eq!(resolve_primary_prio(None), DEFAULT_PRIMARY_PRIO);
    }

    #[test]
    fn a_declaration_inside_the_accepted_range_is_adopted() {
        for p in [0, 1, 100, 1001, 3070, 3071] {
            assert_eq!(resolve_primary_prio(Some(p)), p as u32);
        }
    }

    #[test]
    fn a_declaration_at_or_above_the_ceiling_falls_back_to_the_default() {
        for p in [3072, 3073, 100_000, i32::MAX] {
            assert_eq!(resolve_primary_prio(Some(p)), DEFAULT_PRIMARY_PRIO);
        }
    }

    #[test]
    fn a_negative_declaration_falls_back_instead_of_wrapping() {
        for p in [-1, -512, -513, i32::MIN] {
            assert_eq!(resolve_primary_prio(Some(p)), DEFAULT_PRIMARY_PRIO);
        }
    }
}

mod primary_entry_sp_tests {
    use super::primary_entry_sp;
    use cellgov_ps3_abi::process_address_space::{
        PS3_ABI_MIN_STACK_FRAME as ENTRY_FRAME_RESERVE, PS3_CHILD_STACKS_BASE,
        PS3_PRIMARY_STACK_BASE, PS3_PRIMARY_STACK_SIZE,
    };

    #[test]
    fn the_entry_frame_stays_below_the_child_stack_arena() {
        // The first block `ThreadStackAllocator` hands out starts at
        // PS3_CHILD_STACKS_BASE, so a callee's LR/CR/parameter-save
        // stores above the entry r1 must all land under it.
        let sp = primary_entry_sp();
        assert!(
            sp + ENTRY_FRAME_RESERVE <= PS3_CHILD_STACKS_BASE,
            "entry frame 0x{sp:x}+0x{ENTRY_FRAME_RESERVE:x} reaches the child-stack arena",
        );
        assert!(
            sp + ENTRY_FRAME_RESERVE <= PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64,
            "entry frame escapes the primary stack reservation",
        );
    }

    #[test]
    fn the_entry_sp_is_quadword_aligned() {
        assert_eq!(primary_entry_sp() % 0x10, 0);
    }
}

mod primary_stacksize_tests {
    use super::{decode_primary_stacksize, primary_entry_sp, primary_stack_base_for};
    use cellgov_ps3_abi::process_address_space::{
        PS3_ABI_MIN_STACK_FRAME as ENTRY_FRAME_RESERVE, PS3_PRIMARY_STACK_BASE,
        PS3_PRIMARY_STACK_SIZE,
    };

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
    fn a_raw_byte_count_inside_the_window_passes_through_unchanged() {
        assert_eq!(decode_primary_stacksize(0x10000), 0x10000);
        assert_eq!(decode_primary_stacksize(0x40000), 0x40000);
        assert_eq!(decode_primary_stacksize(0x100000), 0x100000);
    }

    #[test]
    fn a_raw_byte_count_below_the_kernel_floor_is_raised_to_it() {
        // The system software's own param segment declares 0x9000;
        // the floor the kernel enforces is 64 KiB.
        assert_eq!(decode_primary_stacksize(0x9000), 0x10000);
        assert_eq!(decode_primary_stacksize(0), 0x10000);
        assert_eq!(decode_primary_stacksize(1), 0x10000);
        assert_eq!(decode_primary_stacksize(0xFFFF), 0x10000);
    }

    #[test]
    fn a_raw_byte_count_above_the_kernel_ceiling_is_clamped_not_refused() {
        assert_eq!(decode_primary_stacksize(0x10_0001), 0x10_0000);
        assert_eq!(decode_primary_stacksize(u32::MAX), 0x10_0000);
    }

    #[test]
    fn a_raw_byte_count_is_rounded_up_to_a_page() {
        assert_eq!(decode_primary_stacksize(0x10001), 0x11000);
        assert_eq!(decode_primary_stacksize(0x40001), 0x41000);
        // The round-up cannot escape the ceiling, which is itself a
        // page multiple.
        assert_eq!(decode_primary_stacksize(0xF_FFFF), 0x10_0000);
    }

    #[test]
    fn the_recorded_stack_range_always_contains_the_whole_entry_frame() {
        for declared in [0u32, 0x9000, 0x10, 0x40, 0x70, 0x40000, 0x100000, u32::MAX] {
            let size = decode_primary_stacksize(declared);
            let base = primary_stack_base_for(size);
            let end = base + u64::from(size);
            let sp = primary_entry_sp();
            assert!(
                (base..end).contains(&sp),
                "declared 0x{declared:x} -> stack 0x{base:x}..0x{end:x} excludes SP 0x{sp:x}",
            );
            assert!(
                sp + ENTRY_FRAME_RESERVE <= end,
                "declared 0x{declared:x} -> the entry frame above SP 0x{sp:x} runs past the \
                 stack end 0x{end:x}",
            );
            assert!(
                base >= PS3_PRIMARY_STACK_BASE,
                "declared 0x{declared:x} -> base 0x{base:x} escapes the reservation",
            );
            assert_eq!(
                end,
                PS3_PRIMARY_STACK_BASE + PS3_PRIMARY_STACK_SIZE as u64,
                "the stack always ends at the top of the reservation",
            );
        }
    }
}
