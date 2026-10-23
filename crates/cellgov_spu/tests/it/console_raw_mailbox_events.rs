//! CellGov's SPU raises the Le and Me events of the
//! `spu_raw_mailbox_events` probe where the reference console does:
//!
//! - A new SPU starts with Le pending, and with Me and A clear.
//! - Each event rises when the PPU reads its full outbound mailbox.
//!
//! The words come from the probe's console capture.

use cellgov_core::Runtime;
use cellgov_event::UnitId;
use cellgov_exec::UnitStatus;
use cellgov_mem::GuestMemory;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// `tests/micro/spu_raw_mailbox_events/spu/main.S` as the SPU assembler
/// encodes it (`spu-objdump` of the built image), block by block.
const BLOCKS: [(usize, &[u32]); 3] = [
    (
        0x000,
        &[
            0x01a0_0583,
            0x2084_0003,
            0x4084_0005,
            0x21a0_0085,
            0x01e0_0003,
            0x2084_0203,
            0x4080_6005,
            0x21a0_0085,
            0x01e0_0003,
            0x2084_0403,
            0x4080_0004,
            0x2000_0183,
            0x01a0_0004,
            0x21a0_0104,
            0x2084_0604,
            0x40ff_ff86,
            0x21a0_0106,
            0x4108_8887,
            0x6088_8887,
            0x21a0_0e07,
            0x01e0_0003,
            0x2084_0803,
            0x0000_2001,
        ],
    ),
    (
        0x100,
        &[
            0x01e0_0003,
            0x2084_0a03,
            0x4080_0004,
            0x2000_0183,
            0x01a0_0004,
            0x21a0_0104,
            0x2084_0c04,
            0x4111_1107,
            0x6091_1107,
            0x21a0_0f07,
            0x01e0_0003,
            0x2084_0e03,
            0x0000_2002,
        ],
    ),
    (
        0x200,
        &[
            0x01e0_0003,
            0x2084_1003,
            0x4080_0004,
            0x2000_0183,
            0x01a0_0004,
            0x21a0_0104,
            0x2084_1204,
            0x4084_0005,
            0x21a0_0085,
            0x01e0_0003,
            0x2084_1403,
            0x0000_2003,
        ],
    ),
];

/// Where the program stores its words: the preferred slot of the
/// quadword at `WORDS_LS + 16 k`.
const WORDS_LS: usize = 0x2000;
const SPU_WORDS: usize = 11;

const CONSOLE: &str =
    "../../tests/micro/spu_raw_mailbox_events/ps3/cech20-cex-493/observation.json";

fn console_word(at: usize) -> u32 {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 96);
    let word = |at: usize| u32::from_be_bytes(p[4 * at..4 * at + 4].try_into().expect("4 bytes"));
    assert_eq!((word(0), word(1)), (0, 0), "the probe ran every step");
    word(at)
}

fn run_to_stop(rt: &mut Runtime, unit: UnitId) {
    for _ in 0..20 {
        if rt.registry().effective_status(unit) == Some(UnitStatus::Finished) {
            return;
        }
        let step = rt.step().expect("the SPU runs");
        rt.commit_unchecked(&step.result, &step.effects)
            .expect("the step commits");
    }
    panic!("the SPU never stopped");
}

fn spu(rt: &Runtime, unit: UnitId) -> &SpuExecutionUnit {
    rt.registry()
        .get(unit)
        .and_then(|unit| unit.as_any().downcast_ref::<SpuExecutionUnit>())
        .expect("the SPU unit")
}

#[test]
fn the_probe_raises_the_consoles_mailbox_events() {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 100);
    let unit = rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        for (base, words) in BLOCKS {
            for (i, word) in words.iter().enumerate() {
                state.ls[base + i * 4..base + i * 4 + 4].copy_from_slice(&word.to_be_bytes());
            }
        }
        // A raw SPU's interrupt mailbox has a reader: the PPU's
        // sys_raw_spu_read_puint_mb.
        state.channels.out_intr_mbox_connected = true;
        spu
    });

    run_to_stop(&mut rt, unit);
    assert_eq!(rt.read_unit_out_mbox(unit), Ok(Some(console_word(7))));
    rt.write_unit_npc(unit, 0x100).expect("the SPU is stopped");
    rt.restart_unit(unit).expect("the SPU restarts");
    run_to_stop(&mut rt, unit);
    assert_eq!(rt.read_unit_out_intr_mbox(unit), Ok(Some(console_word(9))));
    rt.write_unit_npc(unit, 0x200).expect("the SPU is stopped");
    rt.restart_unit(unit).expect("the SPU restarts");
    run_to_stop(&mut rt, unit);

    let ls = &spu(&rt, unit).state().ls;
    let cellgov: Vec<u32> = (0..SPU_WORDS)
        .map(|k| {
            let at = WORDS_LS + 16 * k;
            u32::from_be_bytes(ls[at..at + 4].try_into().expect("4 bytes"))
        })
        .collect();
    let console: Vec<u32> = (0..SPU_WORDS).map(|k| console_word(11 + k)).collect();
    assert_eq!(
        cellgov, console,
        "(mask, A count, Le|Me count, status, count full, count after the read, \
         status, count full, count after the read, status, A count)"
    );
}
