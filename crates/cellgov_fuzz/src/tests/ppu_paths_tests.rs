use super::*;

use std::error::Error as _;

use cellgov_ppu::observation::PpuObservationComponent;

const ALL_PATHS: [PpuExecutionPath; 4] = [
    PpuExecutionPath::Plain,
    PpuExecutionPath::Forwarded,
    PpuExecutionPath::Quickened,
    PpuExecutionPath::Fused,
];

fn li(rt: u32, value: u16) -> u32 {
    (14 << 26) | (rt << 21) | u32::from(value)
}

fn addi(rt: u32, ra: u32, value: u16) -> u32 {
    (14 << 26) | (rt << 21) | (ra << 16) | u32::from(value)
}

fn stw(rs: u32, ra: u32, offset: u16) -> u32 {
    (36 << 26) | (rs << 21) | (ra << 16) | u32::from(offset)
}

fn sth(rs: u32, ra: u32, offset: u16) -> u32 {
    (44 << 26) | (rs << 21) | (ra << 16) | u32::from(offset)
}

fn lwz(rt: u32, ra: u32, offset: u16) -> u32 {
    (32 << 26) | (rt << 21) | (ra << 16) | u32::from(offset)
}

fn branch(bytes: u32) -> u32 {
    (18 << 26) | (bytes & 0x03ff_fffc)
}

// [PPC-Book1 p:26 s:2.4.2 System Call Instruction] sc SC-form: OPCD 17, LEV at instruction bits 20:26, bit 30 set.
fn sc(lev: u32) -> u32 {
    (17 << 26) | (lev << 5) | 2
}

fn data_state() -> PpuState {
    let mut state = PpuState::new();
    state.set_gpr(4, DATA_BASE);
    state
}

fn baseline_runs() -> Vec<PpuPathRun> {
    run_all_paths(&[li(3, 1)], &PpuState::new(), &[0; 64]).expect("baseline runs")
}

#[test]
fn path_error_display_names_every_refusal() {
    assert_eq!(
        PpuPathError::Memory(MemError::LengthMismatch).to_string(),
        "PPU path memory failed: byte buffer length disagrees with range length"
    );
    assert_eq!(
        PpuPathError::Observation(PpuObservationError::FaultEffectWithoutFault).to_string(),
        "PPU path observation failed: FaultRaised reached a non-faulting PPU observation"
    );
    assert_eq!(
        PpuPathError::CommitRefusal {
            path: PpuExecutionPath::Fused,
            error: PpuObservationError::FaultEffectWithoutFault,
            staged_effects: Vec::new(),
            committed_effects: Vec::new(),
        }
        .to_string(),
        "PPU path Fused commit refused: FaultRaised reached a non-faulting PPU observation"
    );
    assert_eq!(
        PpuPathError::ForwardingWidth { length: 32 }.to_string(),
        "PPU path forwarding write has unsupported width 32"
    );
    assert_eq!(
        PpuPathError::ForwardingCapacity { addr: 0x10 }.to_string(),
        "PPU path forwarding buffer is full at address 0x0000000000000010"
    );
    let refused = PpuPathError::ForwardingRefused {
        source: cellgov_ppu::store_buffer::StoreRefusal::AddressWraps {
            addr: u64::MAX,
            len: 2,
        },
    };
    assert_eq!(
        refused.to_string(),
        "PPU path forwarding refused a write: store at 0xffffffffffffffff of 2 bytes wraps the address space"
    );
    assert!(refused.source().is_some());
    assert_eq!(
        PpuPathError::EmptySequence.to_string(),
        "PPU path sequence must contain at least one instruction"
    );
    assert_eq!(
        PpuPathError::EmptyData.to_string(),
        "PPU path data region must contain at least one byte"
    );
    assert_eq!(
        PpuPathError::RangeOverflow {
            base: DATA_BASE,
            size: 7
        }
        .to_string(),
        "PPU path range at 0x0000000010000000 of 7 bytes overflows"
    );
    assert_eq!(
        PpuPathError::BudgetOverflow { words: 9 }.to_string(),
        "PPU path sequence of 9 instructions exceeds the guest budget range"
    );
    assert_eq!(
        PpuPathError::CodeMutationOutOfRange {
            word_index: 2,
            words: 2
        }
        .to_string(),
        "PPU path code mutation word 2 is outside 2 words"
    );
}

#[test]
fn wrapped_errors_expose_their_source() {
    let commit = PpuPathError::CommitRefusal {
        path: PpuExecutionPath::Plain,
        error: PpuObservationError::FaultEffectWithoutFault,
        staged_effects: Vec::new(),
        committed_effects: Vec::new(),
    };
    assert_eq!(
        commit.source().map(ToString::to_string),
        Some("FaultRaised reached a non-faulting PPU observation".to_owned())
    );
    assert_eq!(
        PpuPathError::Memory(MemError::OverlappingRegions)
            .source()
            .map(ToString::to_string),
        Some("overlapping address ranges".to_owned())
    );
    assert_eq!(
        PpuPathError::Observation(PpuObservationError::FaultEffectWithoutFault)
            .source()
            .map(ToString::to_string),
        Some("FaultRaised reached a non-faulting PPU observation".to_owned())
    );
    assert!(PpuPathError::EmptySequence.source().is_none());
    assert!(PpuPathError::ForwardingWidth { length: 0 }
        .source()
        .is_none());
}

#[test]
fn empty_inputs_are_refused_in_sequence_then_data_order() {
    let state = PpuState::new();
    assert!(matches!(
        run_all_paths(&[], &state, &[]),
        Err(PpuPathError::EmptySequence)
    ));
    assert!(matches!(
        run_all_paths(&[], &state, &[0]),
        Err(PpuPathError::EmptySequence)
    ));
    assert!(matches!(
        run_all_paths(&[li(3, 1)], &state, &[]),
        Err(PpuPathError::EmptyData)
    ));
    for path in ALL_PATHS {
        assert!(
            matches!(
                run_one_path(path, &[], &state, &[]),
                Err(PpuPathError::EmptySequence)
            ),
            "{path:?}"
        );
        assert!(
            matches!(
                run_one_path(path, &[li(3, 1)], &state, &[]),
                Err(PpuPathError::EmptyData)
            ),
            "{path:?}"
        );
    }
}

#[test]
fn code_mutation_index_is_checked_before_the_sequence_and_data() {
    let state = PpuState::new();
    assert!(matches!(
        run_all_paths_after_code_mutation(&[], &state, &[], 0, li(3, 1)),
        Err(PpuPathError::CodeMutationOutOfRange {
            word_index: 0,
            words: 0
        })
    ));
    assert!(matches!(
        run_all_paths_after_code_mutation(&[li(3, 1), li(3, 2)], &state, &[0], 2, li(3, 3)),
        Err(PpuPathError::CodeMutationOutOfRange {
            word_index: 2,
            words: 2
        })
    ));
    assert!(matches!(
        run_all_paths_after_code_mutation(&[li(3, 1)], &state, &[], 0, li(3, 3)),
        Err(PpuPathError::EmptyData)
    ));
}

#[test]
fn run_all_paths_covers_every_path_in_order_and_matches_run_one_path() {
    let state = data_state();
    let words = [li(3, 5), stw(3, 4, 0), lwz(6, 4, 0)];
    let runs = run_all_paths(&words, &state, &[0; 64]).expect("all paths");
    assert_eq!(
        runs.iter().map(|run| run.path).collect::<Vec<_>>(),
        ALL_PATHS
    );
    for (run, path) in runs.iter().zip(ALL_PATHS) {
        let single = run_one_path(path, &words, &state, &[0; 64]).expect("one path");
        assert_eq!(&single, run, "{path:?}");
    }
    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
}

#[test]
fn plain_path_retires_each_word_and_reports_the_final_pc() {
    let runs = run_all_paths(&[li(3, 7), addi(3, 3, 1)], &PpuState::new(), &[0; 64]).expect("runs");
    let plain = &runs[0];
    assert_eq!(plain.path, PpuExecutionPath::Plain);
    assert_eq!(plain.retired, 2);
    assert_eq!(plain.executed_pcs, [0, 4]);
    assert_eq!(plain.observation.state.gpr[3], 8);
    assert_eq!(
        plain.stop,
        PpuPathStop {
            reason: YieldReason::BudgetExhausted,
            fault: None,
            pc: Some(4),
            diagnostics: LocalDiagnostics::with_pc(4),
            syscall_args: None,
        }
    );
    assert!(!plain.observation.fault_discarded);
    assert!(plain.committed_data_reads.is_empty());
    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
}

#[test]
fn plain_path_pads_the_sequence_with_no_ops_so_a_skip_branch_stays_in_code() {
    let words = [branch(8), li(3, 1), li(3, 2)];
    let runs = run_all_paths(&words, &PpuState::new(), &[0; 64]).expect("runs");
    let plain = &runs[0];
    assert_eq!(plain.executed_pcs, [0, 8, 12]);
    assert_eq!(plain.retired, 3);
    assert_eq!(plain.observation.state.gpr[3], 2);
    assert_eq!(plain.stop.reason, YieldReason::BudgetExhausted);
    assert_eq!(plain.stop.pc, Some(12));
    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
}

#[test]
fn plain_path_decode_fault_rolls_back_and_names_the_word() {
    let undecodable = 0;
    assert!(decode(undecodable).is_err());
    let mut state = PpuState::new();
    state.set_lr(0x77);
    let runs = run_all_paths(&[li(3, 7), undecodable], &state, &[0; 64]).expect("runs");
    let plain = &runs[0];
    assert_eq!(plain.stop.reason, YieldReason::Fault);
    assert_eq!(
        plain.stop.fault,
        Some(FaultKind::Guest(cellgov_ppu::FAULT_DECODE_ERROR))
    );
    assert_eq!(plain.stop.pc, Some(4));
    assert_eq!(plain.executed_pcs, [0]);
    assert_eq!(plain.retired, 0);
    assert_eq!(plain.observation.state.gpr[3], 0);
    assert!(plain.observation.fault_discarded);
    assert_eq!(plain.stop.diagnostics.faulting_ea, None);
    assert_eq!(plain.stop.diagnostics.lr, Some(0x77));
    assert_eq!(
        plain
            .stop
            .diagnostics
            .fault_regs
            .as_ref()
            .map(|regs| regs.gprs[3]),
        Some(7)
    );
    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
}

#[test]
fn unmapped_load_reports_the_invalid_address_fault_code_on_every_path() {
    let mut state = PpuState::new();
    state.set_gpr(6, 0x2000_0000);
    let runs = run_all_paths(&[li(3, 7), lwz(4, 6, 0)], &state, &[0; 64]).expect("runs");
    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    for run in &runs {
        assert_eq!(run.stop.reason, YieldReason::Fault, "{:?}", run.path);
        assert_eq!(
            run.stop.fault,
            Some(FaultKind::Guest(cellgov_ppu::FAULT_INVALID_ADDRESS)),
            "{:?}",
            run.path
        );
        assert_eq!(run.stop.pc, Some(4));
        assert_eq!(run.stop.diagnostics.faulting_ea, Some(0x2000_0000));
        assert_eq!(run.retired, 0);
        assert_eq!(run.observation.state.gpr[3], 0);
        assert!(run.committed_data_reads.is_empty());
    }
    assert_eq!(runs[0].executed_pcs, [0, 4]);
}

#[test]
fn syscall_stops_before_later_words_on_every_path() {
    let mut state = PpuState::new();
    state.set_gpr(11, 0x2a);
    state.set_gpr(3, 0x1b);
    let runs = run_all_paths(&[sc(0), li(3, 7)], &state, &[0; 64]).expect("runs");
    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    for run in &runs {
        assert_eq!(run.stop.reason, YieldReason::Syscall, "{:?}", run.path);
        assert_eq!(run.stop.fault, None);
        assert_eq!(run.stop.pc, Some(0));
        assert_eq!(run.stop.diagnostics.syscall_lev, Some(0));
        assert_eq!(
            run.stop.syscall_args.map(|args| [args[0], args[1]]),
            Some([0x2a, 0x1b])
        );
        assert_eq!(run.observation.state.gpr[3], 0x1b);
        assert_eq!(run.executed_pcs, [0], "{:?}", run.path);
        assert!(!run.observation.fault_discarded);
    }
}

#[test]
fn plain_path_refuses_the_store_that_overflows_its_forwarding_buffer() {
    let state = data_state();
    let full = vec![stw(3, 4, 0); 64];
    let run = run_one_path(PpuExecutionPath::Plain, &full, &state, &[0; 64])
        .expect("sixty-four stores fit the forwarding buffer");
    assert_eq!(run.retired, 64);
    let overflow = vec![stw(3, 4, 0); 65];
    assert!(matches!(
        run_one_path(PpuExecutionPath::Plain, &overflow, &state, &[0; 64]),
        Err(PpuPathError::ForwardingCapacity { addr: DATA_BASE })
    ));
    assert!(matches!(
        run_all_paths(&overflow, &state, &[0; 64]),
        Err(PpuPathError::ForwardingCapacity { addr: DATA_BASE })
    ));
}

#[test]
fn a_load_the_latest_store_only_partly_covers_is_a_committed_read_on_every_path() {
    let state = data_state();
    let covered = run_all_paths(
        &[li(3, 0x1111), stw(3, 4, 0), lwz(6, 4, 0)],
        &state,
        &[0; 64],
    )
    .expect("covered runs");
    assert!(first_path_divergence(&covered).is_none(), "{covered:#?}");
    for run in &covered {
        assert_eq!(run.observation.state.gpr[6], 0x1111, "{:?}", run.path);
        assert!(run.committed_data_reads.is_empty(), "{:?}", run.path);
    }

    // The halfword store is the most recent store that touches the word
    // load, and it covers only two of its four bytes. The older word
    // store forwards nothing, so the load reads committed memory on
    // every path.
    let words = [
        li(3, 0x1111),
        stw(3, 4, 0),
        li(5, 0x2222),
        sth(5, 4, 0),
        lwz(6, 4, 0),
    ];
    let runs = run_all_paths(&words, &state, &[0; 64]).expect("partly covered runs");
    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    let load = ByteRange::new(GuestAddr::new(DATA_BASE), 4).expect("fits");
    for run in &runs {
        assert_eq!(run.observation.state.gpr[6], 0x2222_1111, "{:?}", run.path);
        assert_eq!(run.observation.memory[..4], [0x22u8, 0x22, 0x11, 0x11]);
        assert_eq!(run.committed_data_reads, [load], "{:?}", run.path);
    }
}

#[test]
fn first_path_divergence_reports_only_the_differing_dimension() {
    let mut retired = baseline_runs();
    retired[2].retired += 1;
    assert_eq!(
        first_path_divergence(&retired),
        Some(PpuPathDivergence {
            left: PpuExecutionPath::Plain,
            right: PpuExecutionPath::Quickened,
            observation: BTreeSet::new(),
            stop_differs: false,
            retired_differs: true,
            data_reads_differ: false,
        })
    );
    let mut stop = baseline_runs();
    stop[1].stop.syscall_args = Some([0; 9]);
    assert_eq!(
        first_path_divergence(&stop),
        Some(PpuPathDivergence {
            left: PpuExecutionPath::Plain,
            right: PpuExecutionPath::Forwarded,
            observation: BTreeSet::new(),
            stop_differs: true,
            retired_differs: false,
            data_reads_differ: false,
        })
    );
    let mut reads = baseline_runs();
    reads[3]
        .committed_data_reads
        .push(ByteRange::new(GuestAddr::new(DATA_BASE), 4).expect("fits"));
    assert_eq!(
        first_path_divergence(&reads),
        Some(PpuPathDivergence {
            left: PpuExecutionPath::Plain,
            right: PpuExecutionPath::Fused,
            observation: BTreeSet::new(),
            stop_differs: false,
            retired_differs: false,
            data_reads_differ: true,
        })
    );
    let mut discard = baseline_runs();
    discard[1].observation.fault_discarded = true;
    assert_eq!(
        first_path_divergence(&discard),
        Some(PpuPathDivergence {
            left: PpuExecutionPath::Plain,
            right: PpuExecutionPath::Forwarded,
            observation: BTreeSet::from([PpuObservationComponent::FaultDiscard]),
            stop_differs: false,
            retired_differs: false,
            data_reads_differ: false,
        })
    );
}

#[test]
fn first_path_divergence_scans_pairs_in_caller_order() {
    let runs = baseline_runs();
    assert_eq!(first_path_divergence(&runs), None);
    assert_eq!(first_path_divergence(&[]), None);
    assert_eq!(first_path_divergence(&runs[..1]), None);

    let mut last_only = runs.clone();
    last_only[3].retired += 1;
    let pair = |runs: &[PpuPathRun]| first_path_divergence(runs).map(|d| (d.left, d.right));
    assert_eq!(
        pair(&last_only),
        Some((PpuExecutionPath::Plain, PpuExecutionPath::Fused))
    );
    let mut two_later = runs.clone();
    two_later[2].retired += 1;
    two_later[3].retired += 1;
    assert_eq!(
        pair(&two_later),
        Some((PpuExecutionPath::Plain, PpuExecutionPath::Quickened))
    );
    assert_eq!(pair(&two_later[2..]), None);
    assert_eq!(
        pair(&two_later[1..]),
        Some((PpuExecutionPath::Forwarded, PpuExecutionPath::Quickened))
    );
    let mut first_only = runs;
    first_only[0].retired += 1;
    assert_eq!(
        pair(&first_only),
        Some((PpuExecutionPath::Plain, PpuExecutionPath::Forwarded))
    );
    assert_eq!(pair(&first_only[1..]), None);
}

#[test]
fn code_mutation_executes_the_replacement_on_every_path() {
    let state = data_state();
    let mutated =
        run_all_paths_after_code_mutation(&[li(3, 5), stw(3, 4, 0)], &state, &[0; 64], 0, li(3, 9))
            .expect("mutated runs");
    assert!(first_path_divergence(&mutated).is_none(), "{mutated:#?}");
    assert_eq!(
        mutated.iter().map(|run| run.path).collect::<Vec<_>>(),
        ALL_PATHS
    );
    let direct = run_all_paths(&[li(3, 9), stw(3, 4, 0)], &state, &[0; 64]).expect("direct runs");
    for (mutated, direct) in mutated.iter().zip(&direct) {
        assert_eq!(mutated.observation.state.gpr[3], 9, "{:?}", mutated.path);
        assert_eq!(mutated.observation.memory[..4], 9u32.to_be_bytes());
        assert_eq!(mutated.observation.state, direct.observation.state);
        assert_eq!(mutated.observation.memory, direct.observation.memory);
        assert_eq!(mutated.executed_pcs, [0, 4], "{:?}", mutated.path);
        let direct_pcs: &[u64] = if direct.path == PpuExecutionPath::Fused {
            &[0]
        } else {
            &[0, 4]
        };
        assert_eq!(direct.executed_pcs, direct_pcs, "{:?}", direct.path);
    }
    let last_word =
        run_all_paths_after_code_mutation(&[li(3, 5), li(3, 6)], &state, &[0; 64], 1, li(3, 8))
            .expect("last word mutated");
    assert!(
        first_path_divergence(&last_word).is_none(),
        "{last_word:#?}"
    );
    assert!(last_word
        .iter()
        .all(|run| run.observation.state.gpr[3] == 8));
}

#[test]
fn a_one_byte_data_region_is_observed_whole() {
    let state = PpuState::new();
    let runs = run_all_paths(&[li(3, 1)], &state, &[0xaa]).expect("one data byte");
    assert!(first_path_divergence(&runs).is_none(), "{runs:#?}");
    for run in &runs {
        assert_eq!(run.observation.memory, [0xaa], "{:?}", run.path);
        assert_eq!(run.observation.state.gpr[3], 1);
    }
}
