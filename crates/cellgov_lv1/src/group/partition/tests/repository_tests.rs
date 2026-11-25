use super::*;
use cellgov_event::UnitId;
use cellgov_ps3_abi::lv1::repository::{field, first_field};
use cellgov_time::GuestTicks;

/// A read of `bi.<second>` at `index` in partition `lpar`. Linux
/// `ps3_repository_read_pu_id` puts the PU index in the second word, as
/// `make_field("pu", pu_index)`. r8 to r10 hold values the read must not
/// take as key words.
fn read(lpar: u64, second: &[u8], index: u64) -> HypercallRequest {
    HypercallRequest {
        unit: UnitId::new(0),
        number: 91,
        inputs: [
            lpar,
            first_field(b"bi", 0),
            field(second, index),
            0,
            0,
            0x88,
            0x99,
            8,
        ],
        pc: 0x8000_0000_0000_ce3c,
        lr: 0,
        thread: Some(1),
        problem_state: false,
        tick: GuestTicks::ZERO,
    }
}

/// A firmware the psdevwiki dump's values are served to.
const DUMP_FW: Option<&str> = Some("1.50");

/// The repository dump [PS3DEVWIKI-REPO s:3.41_GameOS] gives
/// `bi.pun` = 1.
#[test]
fn a_read_of_bi_pun_in_lpar_2_answers_1_in_r4_and_0_in_r5() {
    assert_eq!(
        read_node(&read(2, b"pun", 0), DUMP_FW),
        HypercallDisposition::Answered(HypercallAnswer::success([1, 0]))
    );
}

#[test]
fn a_read_with_no_firmware_or_an_unnamed_firmware_is_a_refusal_that_names_the_key() {
    let r = read(2, b"pun", 0);
    let refused = HypercallDisposition::Refused(HypercallRefusal::RepositoryNode {
        key: RepositoryKey {
            lpar: 2,
            n1: 0x6269_0000,
            n2: 0x7075_6e00_0000_0000,
            n3: 0,
            n4: 0,
        },
        unit: r.unit,
        pc: r.pc,
    });
    assert_eq!(read_node(&r, None), refused);
    assert_eq!(read_node(&r, Some("3.41")), refused);
}

#[test]
fn bi_pu_index_0_and_index_1_are_two_keys() {
    assert_eq!(
        read_node(&read(2, b"pu", 0), DUMP_FW),
        HypercallDisposition::Answered(HypercallAnswer::success([1, 0]))
    );
    let r = read(2, b"pu", 1);
    assert_eq!(
        read_node(&r, DUMP_FW),
        HypercallDisposition::Refused(HypercallRefusal::RepositoryNode {
            key: RepositoryKey {
                lpar: 2,
                n1: 0x6269_0000,
                n2: 0x7075_0000_0000_0001,
                n3: 0,
                n4: 0,
            },
            unit: r.unit,
            pc: r.pc,
        })
    );
}

#[test]
fn a_read_that_names_lpar_0_is_a_refusal_and_never_no_entry() {
    let r = read(0, b"pun", 0);
    assert_eq!(
        read_node(&r, DUMP_FW),
        HypercallDisposition::Refused(HypercallRefusal::RepositoryNode {
            key: RepositoryKey {
                lpar: 0,
                n1: 0x6269_0000,
                n2: 0x7075_6e00_0000_0000,
                n3: 0,
                n4: 0,
            },
            unit: r.unit,
            pc: r.pc,
        })
    );
}
