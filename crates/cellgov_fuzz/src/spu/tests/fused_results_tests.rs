use cellgov_spu::fuzz::{
    sequence_relations, SpuFusedFlow, SpuSequencePartner, SpuSequenceRelation,
    SpuSequenceRelationId as Id,
};
use cellgov_spu::observation::SpuObservationComponent;
use cellgov_spu::state::{SpuState, SPU_REG_COUNT};
use serde_json::{json, Map, Value};

use super::*;

fn relation(id: Id) -> &'static SpuSequenceRelation {
    sequence_relations()
        .iter()
        .find(|row| row.id == id)
        .expect("every id has a row")
}

fn hex(value: [u8; 16]) -> Value {
    json!(format!("{:032x}", u128::from_be_bytes(value)))
}

fn registers(state: &SpuState) -> Value {
    let map: Map<String, Value> = (0..SPU_REG_COUNT)
        .filter(|&register| state.regs[register] != [0; 16])
        .map(|register| (register.to_string(), hex(state.regs[register])))
        .collect();
    Value::Object(map)
}

fn lines(state: &SpuState) -> Value {
    let map: Map<String, Value> = state
        .ls
        .chunks_exact(16)
        .enumerate()
        .filter(|(_, line)| line.iter().any(|&byte| byte != 0))
        .map(|(index, line)| {
            let mut value = [0u8; 16];
            value.copy_from_slice(line);
            (format!("0x{:05x}", index * 16), hex(value))
        })
        .collect();
    Value::Object(map)
}

/// The state the row's own fused reference leaves from `start`.
fn fused(
    relation: &SpuSequenceRelation,
    assignment: &[u8],
    start: &SpuState,
) -> (SpuState, &'static str) {
    let SpuSequencePartner::Fused(fused) = relation.partner else {
        panic!("{:?} has a guest partner", relation.id);
    };
    let mut result = start.clone();
    let flow = match (fused.apply)(&mut result, assignment) {
        SpuFusedFlow::FallThrough => "FallThrough",
        SpuFusedFlow::Taken => "Taken",
    };
    (result, flow)
}

fn entry(
    name: &str,
    relation: &SpuSequenceRelation,
    assignment: &[u8],
    start: &SpuState,
    result: &SpuState,
    flow: &str,
) -> Value {
    json!({
        "name": name,
        "row": format!("{:?}", relation.id),
        "assignment": assignment,
        "registers": registers(start),
        "local_store": lines(start),
        "result": {
            "registers": registers(result),
            "local_store": lines(result),
            "flow": flow,
        },
    })
}

fn file(entries: &[Value]) -> String {
    json!({ "schema_version": 1, "results": entries }).to_string()
}

fn verdicts(text: &str) -> Vec<FusedResultVerdict> {
    check_fused_results(text)
        .expect("the file is in form")
        .into_iter()
        .map(|result| result.verdict)
        .collect()
}

fn words(values: [u32; 4]) -> [u8; 16] {
    std::array::from_fn(|byte| values[byte / 4].to_be_bytes()[byte % 4])
}

/// `ceq c,a,b; ceqi rt,c,0` on c=r3, a=r4, b=r5, rt=r6, with lanes that
/// compare both equal and unequal.
fn not_equal_start() -> (Vec<u8>, SpuState) {
    let mut start = SpuState::new();
    start.set_reg(4, words([1, 2, 3, 4]));
    start.set_reg(5, words([1, 0, 3, 0]));
    (vec![3, 4, 5, 6], start)
}

#[test]
fn a_result_file_that_drops_one_intermediate_of_an_empty_dead_set_row_is_a_finding() {
    let row = relation(Id::CeqNotEqualFused);
    assert!(row.dead.is_empty());
    let (assignment, start) = not_equal_start();
    let (correct, flow) = fused(row, &assignment, &start);
    let mut dropped = correct.clone();
    dropped.set_reg(3, start.regs[3]);
    assert_eq!(
        verdicts(&file(&[
            entry("correct", row, &assignment, &start, &correct, flow),
            entry("dropped", row, &assignment, &start, &dropped, flow),
        ])),
        [
            FusedResultVerdict::Match,
            FusedResultVerdict::Diverged {
                first_component: SpuObservationComponent::Registers,
                registers: vec![3],
            },
        ]
    );
}

#[test]
fn a_multiply_result_without_one_intermediate_is_a_finding_and_the_full_one_is_not() {
    let row = relation(Id::Mpy32);
    let assignment = [10, 11, 12, 13, 14, 15, 16];
    let mut start = SpuState::new();
    start.set_reg(11, words([0x1234_5678, 0xFFFF_0001, 7, 0x8000_0000]));
    start.set_reg(12, words([0x9ABC_DEF0, 3, 0xFFFF_FFFF, 2]));
    let (correct, flow) = fused(row, &assignment, &start);
    assert_ne!(correct.regs[10], [0; 16], "t1 carries a value to drop");
    let mut dropped = correct.clone();
    dropped.set_reg(10, [0; 16]);
    assert_eq!(
        verdicts(&file(&[
            entry("correct", row, &assignment, &start, &correct, flow),
            entry("dropped", row, &assignment, &start, &dropped, flow),
        ])),
        [
            FusedResultVerdict::Match,
            FusedResultVerdict::Diverged {
                first_component: SpuObservationComponent::Registers,
                registers: vec![10],
            },
        ]
    );
}

#[test]
fn a_dead_register_is_left_out_so_the_row_claims_refinement_not_equality() {
    let (assignment, start) = not_equal_start();
    let (correct, flow) = fused(relation(Id::CeqNotEqualFused), &assignment, &start);
    let mut stale = correct.clone();
    stale.set_reg(3, words([0xDEAD_BEEF; 4]));
    let text = |id| {
        file(&[entry(
            "stale",
            relation(id),
            &assignment,
            &start,
            &stale,
            flow,
        )])
    };
    assert_eq!(
        verdicts(&text(Id::CeqNotEqualResultOnly)),
        [FusedResultVerdict::Match]
    );
    assert!(matches!(
        verdicts(&text(Id::CeqNotEqualFused))[..],
        [FusedResultVerdict::Diverged { .. }]
    ));
}

#[test]
fn the_flow_decides_where_the_result_state_stops() {
    let row = relation(Id::BranchOrxBrz);
    let assignment = [20, 21];
    let start = SpuState::new();
    let (result, flow) = fused(row, &assignment, &start);
    assert_eq!(flow, "Taken", "the OR of a zero register is zero");
    assert_eq!(
        verdicts(&file(&[
            entry("taken", row, &assignment, &start, &result, "Taken"),
            entry("fell", row, &assignment, &start, &result, "FallThrough"),
        ])),
        [
            FusedResultVerdict::Match,
            FusedResultVerdict::Diverged {
                first_component: SpuObservationComponent::ProgramCounter,
                registers: Vec::new(),
            },
        ]
    );
}

#[test]
fn a_start_state_outside_the_precondition_is_inapplicable() {
    let row = relation(Id::SelectCeq);
    // c and a share r7, which the select precondition refuses.
    let assignment = [7, 8, 9, 10, 7, 11];
    let start = SpuState::new();
    let (result, flow) = fused(row, &assignment, &start);
    assert_eq!(
        verdicts(&file(&[entry(
            "aliased",
            row,
            &assignment,
            &start,
            &result,
            flow
        )])),
        [FusedResultVerdict::Inapplicable]
    );
}

#[test]
fn a_file_out_of_form_is_refused_before_any_entry_is_checked() {
    let (assignment, start) = not_equal_start();
    let row = relation(Id::CeqNotEqualFused);
    let (result, flow) = fused(row, &assignment, &start);
    let good = entry("one", row, &assignment, &start, &result, flow);
    let with = |key: &str, value: Value| {
        let mut changed = good.clone();
        changed[key] = value;
        changed
    };
    let mut sideways = good.clone();
    sideways["result"]["flow"] = json!("Sideways");
    assert!(matches!(
        check_fused_results(&json!({ "schema_version": 2, "results": [] }).to_string()),
        Err(FusedResultsError::Schema { found: 2 })
    ));
    assert!(matches!(
        check_fused_results(&file(&[with("row", json!("NoSuchRow"))])),
        Err(FusedResultsError::Entry {
            source: CounterexampleError::UnknownRow { .. },
            ..
        })
    ));
    assert!(matches!(
        check_fused_results(&file(&[with("assignment", json!([3, 4, 5]))])),
        Err(FusedResultsError::Entry {
            source: CounterexampleError::Assignment { .. },
            ..
        })
    ));
    assert!(matches!(
        check_fused_results(&file(&[sideways])),
        Err(FusedResultsError::Flow { .. })
    ));
    assert!(matches!(
        check_fused_results(&file(&[good.clone(), good.clone()])),
        Err(FusedResultsError::DuplicateName { .. })
    ));
    assert!(matches!(
        check_fused_results(&file(&[with("extra", json!(1))])),
        Err(FusedResultsError::Json { .. })
    ));
}

#[test]
fn a_store_result_without_its_local_store_line_diverges_in_local_store() {
    // `ai x,y,48; stqd r,32(x)` on x=r20, y=r21, r=r22: the store lands at
    // y + 80.
    let row = relation(Id::SplitAddressStore);
    let assignment = [20, 21, 22];
    let mut start = SpuState::new();
    start.set_reg(21, words([0x1000, 1, 2, 3]));
    start.set_reg(
        22,
        words([0xAAAA_0001, 0xBBBB_0002, 0xCCCC_0003, 0xDDDD_0004]),
    );
    let mut stored = start.clone();
    stored.set_reg(20, words([0x1030, 0x31, 0x32, 0x33]));
    stored.ls[0x1050..0x1060].copy_from_slice(&start.regs[22]);
    let mut unstored = stored.clone();
    unstored.ls[0x1050..0x1060].fill(0);
    assert_eq!(
        verdicts(&file(&[
            entry("stored", row, &assignment, &start, &stored, "FallThrough"),
            entry(
                "unstored",
                row,
                &assignment,
                &start,
                &unstored,
                "FallThrough"
            ),
        ])),
        [
            FusedResultVerdict::Match,
            FusedResultVerdict::Diverged {
                first_component: SpuObservationComponent::LocalStore,
                registers: Vec::new(),
            },
        ]
    );
}
