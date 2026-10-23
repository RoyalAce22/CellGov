//! A list put with a bad second element, as the `spu_list_bad_size` and
//! `spu_list_bad_ea` captures show it, run through the runtime.
//!
//! On the console each list's first element lands and the next two do
//! not, the kernel reports an MFC DMA alignment interrupt through the
//! group's exception event, and the list's tag never completes: the SPU
//! waits on it until the PPU terminates the group. CellGov moves the same
//! bytes, raises the same class, and parks the SPU on the tag.
//!
//! CellGov does not yet deliver the exception event or stop the group;
//! the boot ends on the exception instead.

use cellgov_core::Runtime;
use cellgov_exec::YieldReason;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory};
use cellgov_ps3_abi::hw::spu::{
    MFC_CMD, MFC_PUTL, MFC_RD_TAG_STAT, MFC_TAG_UPDATE_ALL, MFC_WR_TAG_UPDATE,
};
use cellgov_ps3_abi::lv2::spu::{event, group_join_cause};
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// The probe's tag group.
const TAG: u32 = 1;
/// Where the list sits in local store.
const LIST: usize = 0x3800;
/// The buffer the list puts to, in CellGov's guest memory.
const BUFFER: u32 = 0x1000;
/// The buffer offsets the probe reads: element 1's destination, the bad
/// element's two destinations, and element 3's.
const READ_AT: [u32; 4] = [0x100, 0x180, 0x184, 0x200];

/// The capture's answers.
struct Console {
    /// The exception event's source.
    event_source: u64,
    /// The cause in the event's second data word.
    cause: u32,
    /// How the group's join ended.
    join_cause: u32,
    /// The buffer's words at [`READ_AT`].
    landed: [u32; 4],
}

fn console(probe: &str) -> Console {
    let path = format!("../../tests/micro/{probe}/ps3/cech20-cex-493/observation.json");
    let observation = cellgov_compare::baseline::load(std::path::Path::new(&path))
        .unwrap_or_else(|e| panic!("{path}: {e}"));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 112);
    let word = |at: usize| u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"));
    assert_eq!(word(0), 0, "no step of the probe failed");
    assert_eq!(word(8), 0, "the event came");
    Console {
        event_source: u64::from(word(16)) << 32 | u64::from(word(20)),
        cause: word(36),
        join_cause: word(56),
        landed: [96, 100, 104, 108].map(word),
    }
}

/// Runs the probe's list on one SPU: element 1 of 16 bytes to buffer +
/// 0x100, element 2 of `bad_size` bytes to buffer + `bad_offset`,
/// element 3 of 16 bytes to buffer + 0x200, from local store `src`,
/// every word of which holds its own address. Returns the SPU's yields,
/// the exception's class bit, and the buffer's words at [`READ_AT`].
fn run(src: u32, bad_size: u32, bad_offset: u32) -> (Vec<YieldReason>, Option<u64>, [u32; 4]) {
    let mut rt = Runtime::new(GuestMemory::new(0x8000), Budget::new(100), 400);
    let unit = rt.register_unit_with(|id| {
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
        for at in (src..src + 0x4040).step_by(4) {
            let at = at as usize;
            state.ls[at..at + 4].copy_from_slice(&(at as u32).to_be_bytes());
        }
        for (i, (size, offset)) in [(16, 0x100), (bad_size, bad_offset), (16, 0x200)]
            .into_iter()
            .enumerate()
        {
            let at = LIST + 8 * i;
            state.ls[at..at + 4].copy_from_slice(&size.to_be_bytes());
            state.ls[at + 4..at + 8].copy_from_slice(&(BUFFER + offset).to_be_bytes());
        }
        state.set_reg_word_splat(2, MFC_PUTL);
        state.set_reg_word_splat(7, MFC_TAG_UPDATE_ALL);
        let c = &mut state.channels;
        c.mfc_lsa = src;
        c.mfc_eal = LIST as u32;
        c.mfc_size = 24;
        c.mfc_tag_id = TAG;
        c.tag_mask = 1 << TAG;
        spu
    });
    let mut reasons = Vec::new();
    while let Ok(step) = rt.step() {
        reasons.push(step.result.yield_reason);
        if rt.commit_unchecked(&step.result, &step.effects).is_err() {
            break;
        }
    }
    let exception = rt.take_mfc_exception().inspect(|exception| {
        assert_eq!(exception.unit, unit);
    });
    let class = exception.and_then(|exception| exception.command.error.class().class0_status_bit());
    let landed = READ_AT.map(|offset| {
        let range =
            ByteRange::new(GuestAddr::new(u64::from(BUFFER + offset)), 4).expect("in range");
        let bytes = rt.memory().read(range).expect("the buffer");
        u32::from_be_bytes(bytes.try_into().expect("4 bytes"))
    });
    (reasons, class, landed)
}

fn check(probe: &str, bad_size: u32, bad_offset: u32) {
    let console = console(probe);
    assert_eq!(console.event_source, event::GROUP_EXCEPTION_KEY);
    assert_eq!(console.cause, event::EXCEPTION_DMA_ALIGNMENT);
    assert_eq!(
        console.join_cause,
        group_join_cause::TERMINATED,
        "the SPU never got past the tag wait"
    );
    // Element 1's first word is the local-store address it came from.
    let src = console.landed[0];
    let (reasons, class, landed) = run(src, bad_size, bad_offset);
    assert_eq!(landed, console.landed, "which elements landed");
    assert_eq!(
        class,
        Some(cellgov_ps3_abi::hw::spu::MFC_CLASS0_ALIGNMENT),
        "a DMA alignment interrupt"
    );
    assert_eq!(
        reasons.last(),
        Some(&YieldReason::ChannelStall),
        "the SPU waits on a tag the list never completes: {reasons:?}"
    );
}

/// [CBEA p:57 s:7.2 Table 7-6] a transfer size above 16 KB is a DMA alignment error.
#[test]
fn an_oversized_element_ends_the_list_as_on_the_console() {
    check("spu_list_bad_size", 0x4010, 0x180);
}

/// [CBEA p:57 s:7.2 Table 7-6] an effective address whose low four bits differ from the local-store address's is a DMA alignment error.
#[test]
fn a_misaligned_element_ends_the_list_as_on_the_console() {
    check("spu_list_bad_ea", 16, 0x184);
}
