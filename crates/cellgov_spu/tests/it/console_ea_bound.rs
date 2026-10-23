//! The SPU effective-address bound as the `spu_ea_bound` capture shows
//! it, run through the runtime.
//!
//! On the console a get or a getllar at an unmapped address raises the
//! MFC data-segment interrupt, and MFC_DAR names the address. This holds
//! above 4 GB (EAH 0x1, 0xFF, 0x3FF, 0x400) and below it. CellGov raises
//! the same class at the same address.

use cellgov_core::Runtime;
use cellgov_dma::MfcCommandError;
use cellgov_mem::GuestMemory;
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_GET, MFC_GETLLAR, MFC_RD_TAG_STAT, MFC_TAG_UPDATE_ALL, MFC_WR_TAG_UPDATE,
};
use cellgov_ps3_abi::lv2::spu::{event, group_join_cause};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

const CONSOLE: &str = "../../tests/micro/spu_ea_bound/ps3/cech20-cex-493/observation.json";

/// The probe's access per case: 0 a get, 1 a getllar.
const ACCESS: [u32; 10] = [0, 0, 0, 0, 1, 1, 1, 1, 0, 0];

/// One case's answers: the event's cause and MFC_DAR, and the join cause.
struct Case {
    cause: u32,
    dar: u64,
    join_cause: u32,
}

fn console() -> Vec<Case> {
    let observation = cellgov_compare::baseline::load(std::path::Path::new(CONSOLE))
        .unwrap_or_else(|e| panic!("{CONSOLE}: {e}"));
    let p = &observation.memory_regions[0].data;
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!(word(0), 0, "no step of the probe failed");
    let count = word(8) as usize;
    assert_eq!(count, ACCESS.len());
    assert_eq!(p.len(), 16 + 48 * count);
    (0..count)
        .map(|k| {
            let at = 16 + 48 * k;
            assert_eq!(word(at), 0, "case {k}: the event came");
            assert_eq!(
                0xFFFF_FFFF_0000_0000 | u64::from(word(at + 4)),
                event::GROUP_EXCEPTION_KEY,
                "case {k}"
            );
            Case {
                cause: word(at + 12),
                dar: u64::from(word(at + 16)) << 32 | u64::from(word(at + 20)),
                join_cause: word(at + 32),
            }
        })
        .collect()
}

/// Runs one access at `ea` on one SPU: a get waits for its tag, a
/// getllar is the one command. Returns the exception the runtime raised.
fn run(access: u32, ea: u64) -> Option<MfcCommandError> {
    let mut rt = Runtime::new(GuestMemory::new(0x1000), Budget::new(100), 400);
    rt.register_unit_with(|id| {
        let mut spu = SpuExecutionUnit::new(id);
        let state = spu.state_mut();
        let program = [
            (0x10D << 21) | (u32::from(MFC_CMD) << 7) | 2,
            (0x10D << 21) | (u32::from(MFC_WR_TAG_UPDATE) << 7) | 7,
            (0x00D << 21) | (u32::from(MFC_RD_TAG_STAT) << 7) | 5,
            0,
        ];
        for (i, word) in program.iter().enumerate() {
            state.ls[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
        }
        let (command, size) = if access == 0 {
            (MFC_GET, 16)
        } else {
            (MFC_GETLLAR, 128)
        };
        state.set_reg_word_splat(2, command);
        state.set_reg_word_splat(7, MFC_TAG_UPDATE_ALL);
        let c = &mut state.channels;
        c.mfc_lsa = 0x400;
        c.mfc_eah = (ea >> 32) as u32;
        c.mfc_eal = ea as u32;
        c.mfc_size = size;
        c.mfc_tag_id = 1;
        c.tag_mask = 1 << 1;
        spu
    });
    while let Ok(step) = rt.step() {
        if rt.commit_unchecked(&step.result, &step.effects).is_err() {
            break;
        }
    }
    rt.take_mfc_exception()
        .map(|exception| exception.command.error)
}

/// [CBEA p:120 s:9.1.7] a segment fault suspends the queue and raises the MFC data-segment interrupt.
#[test]
fn an_access_to_an_unmapped_address_is_a_data_segment_fault_as_on_the_console() {
    let cases = console();
    for (k, (case, &access)) in cases.iter().zip(&ACCESS).enumerate() {
        assert_eq!(case.cause, event::EXCEPTION_MFC_SEGMENT, "case {k}");
        assert_eq!(case.join_cause, group_join_cause::TERMINATED, "case {k}");
        assert_eq!(
            run(access, case.dar),
            Some(MfcCommandError::DataSegment { ea: case.dar }),
            "case {k}: access {access} at 0x{:x}",
            case.dar
        );
    }
}
