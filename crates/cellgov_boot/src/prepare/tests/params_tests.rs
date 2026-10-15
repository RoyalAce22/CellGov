//! Process-parameter decode rules: step budget, priority, stack.

mod step_call_cap_tests {
    use super::super::step_call_cap;

    #[test]
    fn an_exact_multiple_reaches_every_requested_instruction() {
        // The boot bench default: 100M instructions at the
        // architectural 256-instruction step grant.
        assert_eq!(step_call_cap(100_000_000, 256), (390_625, 100_000_000));
        assert_eq!(step_call_cap(256, 256), (1, 256));
    }

    #[test]
    fn a_remainder_is_unreachable_and_shows_in_the_effective_cap() {
        // The boot run default: 100_000 is not a multiple of 256, so
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

mod primary_prio_tests {
    use super::super::{resolve_primary_prio, DEFAULT_PRIMARY_PRIO};
    use crate::NullSink;

    #[test]
    fn an_absent_param_segment_takes_the_kernel_default() {
        assert_eq!(resolve_primary_prio(None, &NullSink), DEFAULT_PRIMARY_PRIO);
    }

    #[test]
    fn a_declaration_inside_the_accepted_range_is_adopted() {
        for p in [0, 1, 100, 1001, 3070, 3071] {
            assert_eq!(resolve_primary_prio(Some(p), &NullSink), p as u32);
        }
    }

    #[test]
    fn a_declaration_at_or_above_the_ceiling_falls_back_to_the_default() {
        for p in [3072, 3073, 100_000, i32::MAX] {
            assert_eq!(
                resolve_primary_prio(Some(p), &NullSink),
                DEFAULT_PRIMARY_PRIO
            );
        }
    }

    #[test]
    fn a_negative_declaration_falls_back_instead_of_wrapping() {
        for p in [-1, -512, -513, i32::MIN] {
            assert_eq!(
                resolve_primary_prio(Some(p), &NullSink),
                DEFAULT_PRIMARY_PRIO
            );
        }
    }
}

mod primary_entry_sp_tests {
    use super::super::primary_entry_sp;
    use cellgov_ps3_abi::hw::address_space::{
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
    use super::super::{
        decode_primary_stacksize, primary_entry_sp, primary_stack_base_for,
        resolve_primary_stacksize,
    };
    use crate::NullSink;
    use cellgov_ps3_abi::hw::address_space::{
        PS3_ABI_MIN_STACK_FRAME as ENTRY_FRAME_RESERVE, PS3_PRIMARY_STACK_BASE,
        PS3_PRIMARY_STACK_SIZE, PS3_PRIMARY_STACK_SIZE_DEFAULT,
    };

    #[test]
    fn a_count_inside_the_window_is_adopted_and_rounded_up_to_a_page() {
        assert_eq!(decode_primary_stacksize(0x1000), Some(0x1000));
        assert_eq!(decode_primary_stacksize(0x1001), Some(0x2000));
        assert_eq!(decode_primary_stacksize(0xF_FFFF), Some(0x10_0000));
        assert_eq!(decode_primary_stacksize(0x10_0000), Some(0x10_0000));
    }

    #[test]
    fn a_count_outside_the_window_is_not_adopted() {
        for declared in [0, 0x10, 0x70, 0xFFF, 0x10_0001, u32::MAX] {
            assert_eq!(decode_primary_stacksize(declared), None, "0x{declared:x}");
        }
    }

    #[test]
    fn a_count_the_kernel_does_not_adopt_leaves_the_default() {
        assert_eq!(
            resolve_primary_stacksize(Some(0x10_0001), &NullSink),
            PS3_PRIMARY_STACK_SIZE_DEFAULT
        );
        assert_eq!(
            resolve_primary_stacksize(None, &NullSink),
            PS3_PRIMARY_STACK_SIZE_DEFAULT
        );
    }

    #[test]
    fn the_recorded_stack_range_always_contains_the_whole_entry_frame() {
        for declared in [0u32, 0x9000, 0x10, 0x40, 0x70, 0x40000, 0x100000, u32::MAX] {
            let size = resolve_primary_stacksize(Some(declared), &NullSink);
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
