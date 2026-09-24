//! Spawned-child address-space sizing and exit-stub placement.

mod child_exit_stub_addr_tests {
    use super::super::{child_exit_stub_addr, spawned_child_region_size};

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
        out[18..20].copy_from_slice(&21u16.to_be_bytes()); // EM_PPC64
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
    use super::super::spawned_child_region_size;

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
