//! The console capture of the list: the probe's frame read back, and
//! each component compared with the documents and with CellGov.

use super::replay::{read, run_vector, shape_refusal, StoppedVector, SLOTS};
use super::vector::{DocumentedVector, Location, Verdict};

/// Bytes one run of one vector takes in the frame.
const RUN_BYTES: usize = 64;

/// The frame payload's header bytes.
const HEADER_BYTES: usize = 16;

/// The bytes before the payload: the `CGOV` magic and the payload's
/// byte count, as `cgov_cases_tty` in `tests/micro/common/cgov_cases.h`
/// writes them.
const FRAME_PREFIX: usize = 8;

/// Header word 0 of a frame whose cases all ran.
const DONE: u32 = 0x4341_5345;

/// One run of one vector as the console recorded it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConsoleRun {
    /// The record slots in record order; a VR takes two, high half
    /// first.
    pub slots: [u64; SLOTS],
    /// XER.
    pub xer: u64,
    /// CR.
    pub cr: u32,
    /// The FPSCR.
    pub fpscr: u32,
}

/// A probe frame read back: the buffer's address and each vector's two
/// runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConsoleCapture {
    /// The memory buffer's address on the console.
    pub mem_base: u64,
    /// Each vector's two runs, in list order.
    pub runs: Vec<[ConsoleRun; 2]>,
}

/// Each slot location of `vector` with its first record slot; a VR
/// starts on an even slot.
///
/// This is the slot order of the frame layout of
/// `tests/micro/common/cgov_cases.h`.
pub fn slots(vector: &DocumentedVector) -> Vec<(Location, usize)> {
    let mut next = 0;
    vector
        .record()
        .into_iter()
        .map(|at| {
            let width = if matches!(at, Location::Vr(_)) { 2 } else { 1 };
            if width == 2 && next % 2 == 1 {
                next += 1;
            }
            let first = next;
            next += width;
            (at, first)
        })
        .collect()
}

fn word(p: &[u8], at: usize) -> u32 {
    let mut b = [0u8; 4];
    b.copy_from_slice(&p[at..at + 4]);
    u32::from_be_bytes(b)
}

fn dword(p: &[u8], at: usize) -> u64 {
    let mut b = [0u8; 8];
    b.copy_from_slice(&p[at..at + 8]);
    u64::from_be_bytes(b)
}

/// Why a probe frame is not a completed run of the list.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FrameError {
    /// The frame does not start with the `CGOV` magic.
    #[error("frame magic is not CGOV")]
    Magic,
    /// The payload is not the size the list's runs take.
    #[error("payload holds {have} bytes, {want} expected")]
    Size {
        /// The payload's bytes.
        have: usize,
        /// The bytes the list's runs take.
        want: usize,
    },
    /// Header word 0 is not the completed-run status.
    #[error("frame status 0x{status:08x}")]
    Status {
        /// Header word 0.
        status: u32,
    },
    /// Header word 1 does not count the list's vectors.
    #[error("frame counts {count} cases, the list holds {want}")]
    Count {
        /// Header word 1.
        count: u32,
        /// The list's vectors.
        want: usize,
    },
}

/// Read a probe frame of `vectors`.
///
/// # Errors
///
/// A [`FrameError`] when the frame's magic, size, status word or vector
/// count is not that of a completed run of `vectors`.
pub fn read_frame(
    frame: &[u8],
    vectors: &[DocumentedVector],
) -> Result<ConsoleCapture, FrameError> {
    if frame.get(..4) != Some(b"CGOV".as_slice()) {
        return Err(FrameError::Magic);
    }
    let p = &frame[FRAME_PREFIX.min(frame.len())..];
    let want = HEADER_BYTES + 2 * RUN_BYTES * vectors.len();
    if p.len() != want {
        return Err(FrameError::Size {
            have: p.len(),
            want,
        });
    }
    if word(p, 0) != DONE {
        return Err(FrameError::Status { status: word(p, 0) });
    }
    if word(p, 4) as usize != vectors.len() {
        return Err(FrameError::Count {
            count: word(p, 4),
            want: vectors.len(),
        });
    }
    let runs = (0..vectors.len())
        .map(|v| {
            [0, 1].map(|run| {
                let at = HEADER_BYTES + (2 * v + run) * RUN_BYTES;
                ConsoleRun {
                    slots: std::array::from_fn(|s| dword(p, at + 8 * s)),
                    xer: dword(p, at + 32),
                    cr: word(p, at + 40),
                    fpscr: dword(p, at + 48) as u32,
                }
            })
        })
        .collect();
    Ok(ConsoleCapture {
        mem_base: u64::from(word(p, 12)),
        runs,
    })
}

/// The value the console recorded at `at`, from the run's slots.
fn console_value(run: &ConsoleRun, slots: &[(Location, usize)], at: Location) -> u128 {
    match at {
        Location::Cr => u128::from(run.cr),
        Location::Xer => u128::from(run.xer),
        Location::Fpscr => u128::from(run.fpscr),
        _ => {
            let Some(&(_, slot)) = slots.iter().find(|&&(l, _)| l == at) else {
                unreachable!("record() names every slot location a component names")
            };
            match at {
                Location::Vr(_) => {
                    u128::from(run.slots[slot]) << 64 | u128::from(run.slots[slot + 1])
                }
                _ => u128::from(run.slots[slot]),
            }
        }
    }
}

/// One component on which two answers differ.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Disagreement {
    /// The vector's id.
    pub vector: String,
    /// The component's name.
    pub component: String,
    /// What the console recorded, under the mask.
    pub console: u128,
    /// The documents' value or CellGov's, under the mask.
    pub other: u128,
}

impl Disagreement {
    /// `<vector> <component>`, the name a pinned list holds.
    pub fn key(&self) -> String {
        format!("{} {}", self.vector, self.component)
    }
}

/// The capture compared with the documents and with CellGov.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CaptureReport {
    /// Documented components compared.
    pub documented: usize,
    /// Documented components whose console value differs from the
    /// documents' in either run.
    pub against_documents: Vec<Disagreement>,
    /// Undefined components compared.
    pub undefined: usize,
    /// Undefined components the two console runs agree on and CellGov
    /// matches.
    pub agreeing: usize,
    /// Undefined components the two console runs agree on and CellGov
    /// does not match.
    pub contradicted: Vec<Disagreement>,
    /// Undefined components the two console runs differ on, as
    /// `<vector> <component>`.
    pub varies: Vec<String>,
    /// Vectors CellGov did not run to their end.
    pub stopped: Vec<StoppedVector>,
}

/// Compare `capture` with the documents and with CellGov's replay of
/// each vector at the console's buffer address.
///
/// [PPC-Book1 p:3 s:1.5.1] an undefined value may vary between executions, so each undefined component is first checked between the two runs.
pub fn compare_capture(vectors: &[DocumentedVector], capture: &ConsoleCapture) -> CaptureReport {
    let mut report = CaptureReport::default();
    for (vector, runs) in vectors.iter().zip(&capture.runs) {
        let slots = slots(vector);
        let cellgov = run_vector(vector, capture.mem_base);
        if shape_refusal(vector).is_some() {
            if let Err(stopped) = cellgov {
                report.stopped.push(stopped);
            }
            continue;
        }
        for c in &vector.expected {
            let [a, b] = runs.map(|r| console_value(&r, &slots, c.location) & c.mask);
            let key = || format!("{} {}", vector.id, c.name);
            match c.verdict {
                Verdict::Value { value, .. } => {
                    report.documented += 1;
                    let value = value & c.mask;
                    for console in [a, b] {
                        if console != value {
                            report.against_documents.push(Disagreement {
                                vector: vector.id.clone(),
                                component: c.name.clone(),
                                console,
                                other: value,
                            });
                            break;
                        }
                    }
                }
                Verdict::Undefined { .. } => {
                    report.undefined += 1;
                    if a != b {
                        report.varies.push(key());
                        continue;
                    }
                    let Ok((state, mem)) = &cellgov else {
                        continue;
                    };
                    let ours = read(c.location, state, mem) & c.mask;
                    if ours == a {
                        report.agreeing += 1;
                    } else {
                        report.contradicted.push(Disagreement {
                            vector: vector.id.clone(),
                            component: c.name.clone(),
                            console: a,
                            other: ours,
                        });
                    }
                }
            }
        }
        if let Err(stopped) = cellgov {
            report.stopped.push(stopped);
        }
    }
    report
}
