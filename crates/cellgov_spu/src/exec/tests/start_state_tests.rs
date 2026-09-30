//! A fresh SPU starts in the architected start state: every channel
//! count, the channel data, and both signal-notification modes.

use super::*;
use crate::exec::SpuFault;
use crate::state::{SignalNotifyMode, SpuObservableSnapshot, SpuState};

/// Every count a fresh SPU's `rchcnt` reads, by channel number; a
/// channel absent here refuses its count.
// [CBEA p:238 s:16.3.3] the counts of x'0', x'3', x'4', x'18', x'19', x'1B' and x'1D' start at 0; x'17', x'1C' and x'1E' at 1; MFC_Cmd at the queue depth.
// [CBEA p:109 s:9] a nonblocking channel counts 1.
// [CBE-Handbook p:445 s:17.1 Table 17-2] MFC_Cmd holds 16 entries.
const START_COUNTS: [(u8, u32); 18] = [
    (0x00, 0),
    (0x03, 0),
    (0x04, 0),
    (0x0D, 1),
    (0x10, 1),
    (0x11, 1),
    (0x12, 1),
    (0x13, 1),
    (0x14, 1),
    (0x15, 16),
    (0x16, 1),
    (0x17, 1),
    (0x18, 0),
    (0x19, 0),
    (0x1B, 0),
    (0x1C, 1),
    (0x1D, 0),
    (0x1E, 1),
];

#[test]
fn rchcnt_on_a_fresh_unit_reads_the_start_count_of_every_channel_number() {
    let mut expected = [None; 128];
    for (channel, count) in START_COUNTS {
        expected[channel as usize] = Some(count);
    }
    let mut s = SpuState::new();
    for channel in 0..128u8 {
        let out = execute(
            &SpuInstruction::Rchcnt { rt: 3, channel },
            &mut s,
            UnitId::new(0),
        );
        let got = match out {
            SpuStepOutcome::Continue => {
                assert_eq!(s.regs[3][4..], [0; 12], "channel 0x{channel:02x}");
                Some(s.reg_word(3))
            }
            SpuStepOutcome::Fault(SpuFault::UnsupportedChannelCount(c)) if c == channel => None,
            other => panic!("channel 0x{channel:02x}: {other:?}"),
        };
        assert_eq!(got, expected[channel as usize], "channel 0x{channel:02x}");
    }
}

// [CBEA p:237 s:16.3.2] the channel data starts at zero.
// [CBEA p:239 s:16.4] both signal-notification registers start in overwrite mode, the power-on reset value.
// [CBE-Handbook p:421 s:14.6.3.4] the registers and local store start at zero.
#[test]
fn a_fresh_unit_starts_with_zero_data_and_overwrite_signal_modes() {
    let s = SpuObservableSnapshot::capture(&SpuState::new());
    assert_eq!(
        s.signals.map(|r| (r.mode, r.word, r.pending)),
        [(SignalNotifyMode::Overwrite, 0, false); 2]
    );
    let c = &s.channels;
    assert_eq!(
        [
            c.mfc_lsa,
            c.mfc_eah,
            c.mfc_eal,
            c.mfc_size,
            c.mfc_tag_id,
            c.tag_mask,
            c.tag_status,
            c.atomic_status,
            c.in_mbox_count,
        ],
        [0; 9]
    );
    assert_eq!(c.pending_mbox_rt, None);
    assert_eq!(c.out_mbox, None);
    assert_eq!(c.pending_get, None);
    assert!(!c.tag_update_pending);
    assert!(!c.atomic_status_ready);
    assert!(s.regs.iter().all(|r| *r == [0; 16]));
    assert!(s.ls.iter().all(|b| *b == 0));
    assert_eq!((s.pc, s.reservation, s.stop), (0, None, None));
}
