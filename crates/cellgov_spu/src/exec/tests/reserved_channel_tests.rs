//! Reserved channels on the CBE: rdch reads zero, wrch has no effect, and
//! rchcnt counts zero, all without a fault.

// [CBE-Handbook p:443 s:17.1.4] reads of reserved channels return zeros, writes have no effect, and their counts read 0.

use super::*;
use crate::exec::SpuFault;
use crate::state::{SpuObservableSnapshot, SpuState};

/// Every channel number the CBE leaves reserved.
///
/// [CBEA p:299 s:Appendix B, Table B-1] channels 5, 6 and 10 are reserved.
/// [CBE-Handbook p:446 s:17.1.4, Table 17-2] channels 31 to 127 are reserved.
fn reserved() -> impl Iterator<Item = u8> {
    [0x05, 0x06, 0x0A].into_iter().chain(0x1F..=0x7F)
}

#[test]
fn the_reserved_set_has_every_number_the_map_reserves() {
    assert_eq!(reserved().count(), 3 + 97);
    for channel in 0..=0x7F {
        assert_eq!(
            reserved().any(|r| r == channel),
            cellgov_ps3_abi::hw::spu::is_reserved_channel(channel),
            "channel 0x{channel:02x}"
        );
    }
    assert!((0x80..=u8::MAX).all(|c| !cellgov_ps3_abi::hw::spu::is_reserved_channel(c)));
}

#[test]
fn rdch_of_a_reserved_channel_reads_zero() {
    for channel in reserved() {
        let mut s = SpuState::new();
        s.regs[7] = [0xA5; 16];
        let out = execute(
            &SpuInstruction::Rdch { rt: 7, channel },
            &mut s,
            UnitId::new(0),
        );
        assert!(
            matches!(out, SpuStepOutcome::Continue),
            "channel 0x{channel:02x}: {out:?}"
        );
        assert_eq!(s.regs[7], [0; 16], "channel 0x{channel:02x}");
    }
}

#[test]
fn wrch_to_a_reserved_channel_changes_nothing() {
    for channel in reserved() {
        let mut s = SpuState::new();
        s.regs[7] = [0xA5; 16];
        let before = SpuObservableSnapshot::capture(&s);
        let out = execute(
            &SpuInstruction::Wrch { channel, rt: 7 },
            &mut s,
            UnitId::new(0),
        );
        assert!(
            matches!(out, SpuStepOutcome::Continue),
            "channel 0x{channel:02x}: {out:?}"
        );
        assert_eq!(
            SpuObservableSnapshot::capture(&s),
            before,
            "channel 0x{channel:02x}"
        );
    }
}

#[test]
fn rchcnt_of_a_reserved_channel_counts_zero() {
    for channel in reserved() {
        let mut s = SpuState::new();
        s.regs[7] = [0xA5; 16];
        let out = execute(
            &SpuInstruction::Rchcnt { rt: 7, channel },
            &mut s,
            UnitId::new(0),
        );
        assert!(
            matches!(out, SpuStepOutcome::Continue),
            "channel 0x{channel:02x}: {out:?}"
        );
        assert_eq!(s.regs[7], [0; 16], "channel 0x{channel:02x}");
    }
}

#[test]
fn an_implemented_channel_without_an_arm_still_refuses_by_name() {
    // SPU_RdSigNotify1 is an architected read channel the model has not written.
    let mut s = SpuState::new();
    let out = execute(
        &SpuInstruction::Rdch { rt: 7, channel: 3 },
        &mut s,
        UnitId::new(0),
    );
    assert!(matches!(
        out,
        SpuStepOutcome::Fault(SpuFault::UnsupportedChannel {
            channel: 3,
            is_write: false
        })
    ));
}
