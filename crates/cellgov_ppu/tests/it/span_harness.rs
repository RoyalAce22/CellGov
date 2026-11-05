//! The instruction encoders and the runtime loop that the
//! `ppu_span_shm_maps`, `spu_raw_window_span` and `ppu_spu_reserved_loads`
//! replays share: one PPU thread runs a program of system calls and
//! accesses to its end.

use cellgov_core::{Runtime, StepError};
use cellgov_effects::FaultKind;
use cellgov_event::UnitId;
use cellgov_mem::{ByteRange, GuestAddr, GuestMemory, Region};
use cellgov_ppu::PpuExecutionUnit;
use cellgov_spu::SpuExecutionUnit;
use cellgov_time::Budget;

/// Where the program sits.
pub(crate) const TEXT: u64 = 0x1_0000;
/// Where the calls write their outputs.
pub(crate) const OUT: u64 = 0x800;

/// `sc`.
pub(crate) const SC: u32 = 0x4400_0002;

fn d_form(op: u32, rt: u32, ra: u32, d: i16) -> u32 {
    op << 26 | rt << 21 | ra << 16 | u32::from(d as u16)
}
pub(crate) fn li(rt: u32, si: i16) -> u32 {
    d_form(14, rt, 0, si)
}
pub(crate) fn addi(rt: u32, ra: u32, si: i16) -> u32 {
    d_form(14, rt, ra, si)
}
pub(crate) fn addis(rt: u32, ra: u32, si: u16) -> u32 {
    d_form(15, rt, ra, si as i16)
}
pub(crate) fn ori(ra: u32, rs: u32, ui: u16) -> u32 {
    d_form(24, rs, ra, ui as i16)
}
pub(crate) fn oris(ra: u32, rs: u32, ui: u16) -> u32 {
    d_form(25, rs, ra, ui as i16)
}
pub(crate) fn lwz(rt: u32, d: i16, ra: u32) -> u32 {
    d_form(32, rt, ra, d)
}
pub(crate) fn stw(rs: u32, d: i16, ra: u32) -> u32 {
    d_form(36, rs, ra, d)
}
pub(crate) fn ld(rt: u32, ds: i16, ra: u32) -> u32 {
    d_form(58, rt, ra, ds)
}
pub(crate) fn std(rs: u32, ds: i16, ra: u32) -> u32 {
    d_form(62, rs, ra, ds)
}
/// `sldi ra, rs, 32`.
pub(crate) fn sldi32(ra: u32, rs: u32) -> u32 {
    30 << 26 | rs << 21 | ra << 16 | 62 << 5 | 1 << 2 | 1 << 1
}
/// `mr ra, rs`.
fn mr(ra: u32, rs: u32) -> u32 {
    31 << 26 | rs << 21 | ra << 16 | rs << 11 | 444 << 1
}

/// Loads the 32-bit `value` into `rt`; the high word takes its sign.
pub(crate) fn word(p: &mut Vec<u32>, rt: u32, value: u32) {
    p.push(addis(rt, 0, (value >> 16) as u16));
    p.push(ori(rt, rt, value as u16));
}

/// Loads 0x1122334455667788 into `rt`.
pub(crate) fn doubleword(p: &mut Vec<u32>, rt: u32) {
    word(p, rt, 0x1122_3344);
    p.push(sldi32(rt, rt));
    p.push(oris(rt, rt, 0x5566));
    p.push(ori(rt, rt, 0x7788));
}

/// Runs `args`, makes the call `num` and copies its return code to `rc`.
pub(crate) fn call(p: &mut Vec<u32>, num: u64, args: &[u32], rc: u32) {
    p.extend_from_slice(args);
    p.push(li(11, num as i16));
    p.push(SC);
    p.push(mr(rc, 3));
}

/// Ends the program with `sys_process_exit`.
pub(crate) fn exit(p: &mut Vec<u32>) {
    p.push(li(11, cellgov_ps3_abi::lv2::syscall::PROCESS_EXIT as i16));
    p.push(SC);
}

/// The runtime after a run, with its PPU and its first fault.
pub(crate) struct Ran {
    pub(crate) rt: Runtime,
    pub(crate) ppu: UnitId,
    pub(crate) first_fault: Option<(UnitId, FaultKind)>,
}

impl Ran {
    /// The PPU's general-purpose registers.
    pub(crate) fn gpr(&self) -> [u64; 32] {
        let unit = self.rt.registry().get(self.ppu).expect("the PPU").as_any();
        let state = unit
            .downcast_ref::<PpuExecutionUnit>()
            .expect("a PPU unit")
            .state();
        std::array::from_fn(|i| state.gpr[i])
    }
}

/// Runs `program` at [`TEXT`] in a PPU over `regions`, with SPU units for
/// any Raw SPU it creates, until no unit can step.
pub(crate) fn run(regions: Vec<Region>, program: &[u32]) -> Ran {
    let mut memory = GuestMemory::from_regions(regions).expect("the regions tile");
    for (i, w) in program.iter().enumerate() {
        let range = ByteRange::new(GuestAddr::new(TEXT + 4 * i as u64), 4).expect("range");
        memory
            .apply_commit(range, &w.to_be_bytes())
            .expect("mapped");
    }
    let mut rt = Runtime::new(memory, Budget::new(64), 10_000);
    rt.set_spu_factory(|id, _| Ok(Box::new(SpuExecutionUnit::new(id))));
    let ppu = rt.register_unit_with(|id| {
        let mut unit = PpuExecutionUnit::new(id);
        unit.state_mut().pc = TEXT;
        unit
    });
    let mut first_fault = None;
    loop {
        // A run finishes when no unit can step. A step cap or a time
        // overflow stops it early, so those errors panic.
        let mut step = match rt.step() {
            Ok(step) => step,
            Err(StepError::NoRunnableUnit | StepError::AllBlocked) => break,
            Err(e) => panic!("the run stopped short: {e}"),
        };
        if first_fault.is_none() {
            first_fault = step.result.fault.map(|f| (step.unit, f));
        }
        rt.commit_step(&mut step).expect("the step commits");
    }
    Ran {
        rt,
        ppu,
        first_fault,
    }
}

/// A capture's payload as big-endian u32 words.
pub(crate) fn payload(probe: &str, words: usize) -> Vec<u32> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/micro")
        .join(probe)
        .join("ps3/cech20-cex-493/observation.json");
    let observation = cellgov_compare::baseline::load(&path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let p = &observation.memory_regions[0].data;
    assert_eq!(p.len(), 4 * words, "{probe}");
    p.chunks_exact(4)
        .map(|w| u32::from_be_bytes(w.try_into().expect("4")))
        .collect()
}
