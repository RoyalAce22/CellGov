use super::*;

#[test]
fn signed_address_addition_checks_both_directions() {
    assert_eq!(add_signed(0x1000, 0x20), Some(0x1020));
    assert_eq!(add_signed(0x1000, -0x20), Some(0x0fe0));
    assert_eq!(add_signed(0, -1), None);
    assert_eq!(add_signed(u64::MAX, 1), None);
}

#[test]
fn equality_branch_shape_reads_cr7_eq() {
    assert!(matches!(
        crate::decode::decode(0x7c7e_1b78),
        Ok(PpuInstruction::Or {
            ra: 30,
            rs: 3,
            rb: 3,
            rc: false,
        })
    ));
    let compare = crate::decode::decode(0x2fbe_0002).expect("cmpdi r30, 2");
    let branch = crate::decode::decode(0x419e_00a8).expect("beq cr7");
    assert_eq!(unsigned_compare(&compare), Some((7, 30, 2)));
    assert!(branches_on_equal(Some(branch), 7));
}

#[test]
fn relative_jump_table_yields_packet_targets_and_stub_class() {
    let mut elf = vec![0u8; 0x2000];
    elf[0x10f8..0x1100].copy_from_slice(&0x2200u64.to_be_bytes());
    elf[0x1200..0x1204].copy_from_slice(&(-4i32).to_be_bytes());
    elf[0x1204..0x1208].copy_from_slice(&(-4i32).to_be_bytes());
    elf[0x1208..0x120c].copy_from_slice(&8i32.to_be_bytes());
    let segments = [LoadSegment {
        index: 0,
        file_offset: 0x1000,
        vaddr: 0x2000,
        filesz: 0x1000,
        memsz: 0x1000,
        executable: false,
        writable: true,
        readable: true,
    }];
    let decoded = vec![
        Some(PpuInstruction::Cmpldi {
            bf: 7,
            ra: 29,
            imm: 1,
        }),
        Some(PpuInstruction::Bc {
            bo: 12,
            bi: 29,
            offset: 8,
            aa: false,
            link: false,
        }),
        Some(PpuInstruction::Ld {
            rt: 11,
            ra: 2,
            imm: -8,
        }),
        Some(PpuInstruction::Rldicr {
            ra: 9,
            rs: 29,
            sh: 2,
            me: 61,
            rc: false,
        }),
        Some(PpuInstruction::Lwzx {
            rt: 0,
            ra: 9,
            rb: 11,
        }),
        Some(PpuInstruction::Extsw {
            ra: 0,
            rs: 0,
            rc: false,
        }),
        Some(PpuInstruction::Add {
            rt: 0,
            ra: 0,
            rb: 11,
            oe: false,
            rc: false,
        }),
        Some(PpuInstruction::Mtctr { rs: 0 }),
        Some(PpuInstruction::Bcctr {
            bo: 20,
            bi: 0,
            link: false,
        }),
    ];
    let reachable = (0..decoded.len()).collect();
    let found = jump_table_after(
        &elf,
        &segments,
        0x2100,
        &decoded,
        &reachable,
        0,
        29,
        Linear {
            slot: 0,
            delta: -0x2001,
        },
        2,
    )
    .expect("recognize jump table");
    assert_eq!(
        found,
        Lv2Subdispatch::Table {
            selector_slot: 0,
            entries: vec![
                Lv2Subentry {
                    packet: 0x2001,
                    class: Lv2SubentryClass::Stub,
                    target: 0x21fc,
                },
                Lv2Subentry {
                    packet: 0x2002,
                    class: Lv2SubentryClass::Stub,
                    target: 0x21fc,
                },
                Lv2Subentry {
                    packet: 0x2003,
                    class: Lv2SubentryClass::Implemented,
                    target: 0x2208,
                },
            ],
        }
    );
}

#[test]
fn second_stack_prologue_bounds_linear_chain_scan() {
    let decoded = vec![
        Some(PpuInstruction::Stdu {
            rs: 1,
            ra: 1,
            imm: -64,
        }),
        Some(PpuInstruction::Bclr {
            bo: 20,
            bi: 0,
            link: false,
        }),
        Some(PpuInstruction::Stdu {
            rs: 1,
            ra: 1,
            imm: -80,
        }),
    ];
    assert_eq!(function_extent(&decoded), 1);
}

#[test]
fn conditional_indirect_branch_reaches_fallthrough_and_link_clobbers_linears() {
    let decoded = vec![
        Some(PpuInstruction::Bcctr {
            bo: 12,
            bi: 2,
            link: false,
        }),
        Some(PpuInstruction::Or {
            ra: 29,
            rs: 3,
            rb: 3,
            rc: false,
        }),
    ];
    assert_eq!(
        reachable_indices(&decoded, 0x1000),
        [0, 1].into_iter().collect()
    );

    let mut linear = [None; 32];
    for slot in 0..8usize {
        linear[3 + slot] = Some(Linear { slot, delta: 0 });
    }
    linear[13] = Some(Linear { slot: 7, delta: 0 });
    update_linear(
        &mut linear,
        &PpuInstruction::Bcctr {
            bo: 20,
            bi: 0,
            link: true,
        },
    );
    assert!(linear[..13].iter().all(Option::is_none));
    assert!(linear[13].is_some());
}
