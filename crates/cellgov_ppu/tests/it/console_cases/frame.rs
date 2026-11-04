//! The capture reader: a probe's committed frame as runs of record slots.

use super::table::{probe_dir, PROFILE, SLOTS};

/// Bytes one run of one case takes in the frame.
pub(super) const RUN_BYTES: usize = 64;

/// The frame's header bytes.
const HEADER_BYTES: usize = 16;

/// Word 0 of a frame whose cases all ran.
const DONE: u32 = 0x4341_5345;

/// Word 0 of a progress record.
const PROGRESS: u32 = 0x5052_4f47;

/// One run of one case as the console recorded it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Run {
    /// The record slots, in table order; a VR takes two, high half
    /// first, and an unused slot holds 0.
    pub slots: [u64; SLOTS],
    /// XER.
    pub xer: u64,
    /// CR.
    pub cr: u32,
    /// The FPSCR word of the `mffs` result.
    pub fpscr: u32,
}

/// What a probe's committed capture holds.
#[derive(Debug)]
pub enum Capture {
    /// Every case ran: each case's two runs.
    Done(Vec<[Run; 2]>),
    /// The run ended inside the case the progress record names.
    Ended {
        /// The index of that case.
        case: usize,
    },
}

fn payload(probe: &str) -> Vec<u8> {
    let path = probe_dir(probe)
        .join("ps3")
        .join(PROFILE)
        .join("cgov_frame.bin");
    let frame = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    assert_eq!(&frame[0..4], b"CGOV", "{probe}: frame magic");
    frame[8..].to_vec()
}

fn word(p: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(p[at..at + 4].try_into().expect("4 bytes"))
}

fn dword(p: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(p[at..at + 8].try_into().expect("8 bytes"))
}

/// Read the probe's committed capture.
pub fn capture(probe: &str, cases: usize) -> Capture {
    let p = payload(probe);
    assert_eq!(
        p.len(),
        HEADER_BYTES + 2 * RUN_BYTES * cases,
        "{probe}: payload size"
    );
    assert_eq!(word(&p, 4) as usize, cases, "{probe}: case count");
    match word(&p, 0) {
        DONE => {}
        PROGRESS => {
            return Capture::Ended {
                case: word(&p, 8) as usize,
            }
        }
        other => panic!("{probe}: frame status {other:#010x}"),
    }
    let runs = (0..cases)
        .map(|c| {
            [0, 1].map(|run| {
                let at = HEADER_BYTES + (2 * c + run) * RUN_BYTES;
                Run {
                    slots: [0, 1, 2, 3].map(|s| dword(&p, at + 8 * s)),
                    xer: dword(&p, at + 32),
                    cr: word(&p, at + 40),
                    fpscr: dword(&p, at + 48) as u32,
                }
            })
        })
        .collect();
    Capture::Done(runs)
}
