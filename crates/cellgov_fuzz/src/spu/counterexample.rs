//! Stored sequence-relation counterexamples: a start state that separates a
//! row's sequence from its partner, kept as a fixture and replayed first.
//!
//! [Schkufza2013 p:308 s:4.1] A failed equivalence check produces a
//! counterexample testcase, and the testcase set that later checks run
//! grows by it.

use std::collections::BTreeMap;

use cellgov_spu::fuzz::{sequence_relations, SpuSequenceRelation};
use cellgov_spu::observation::SpuObservationComponent;
use cellgov_spu::state::{SpuState, SPU_LS_SIZE, SPU_REG_COUNT};

use super::sequence_relations::{compare_relation, RelationInstance, RelationVerdict};
use crate::error::{FuzzError, InvariantError};
use crate::report::SequenceRelationDivergence;

/// Schema version of a counterexample fixture.
pub(crate) const COUNTEREXAMPLE_SCHEMA_VERSION: u32 = 1;

/// Candidate runs one reduction makes before it keeps what it has.
const REDUCTION_BUDGET: u32 = 4096;

/// Bytes in one local-store line of a fixture.
const LINE: usize = 16;

/// The counterexamples every SPU sequence campaign replays first.
const STORE: &str = include_str!("../../counterexamples/spu_sequence_relations.json");

/// Every observation component, in comparison order, by fixture name.
const COMPONENTS: [(SpuObservationComponent, &str); 11] = [
    (SpuObservationComponent::Registers, "Registers"),
    (SpuObservationComponent::LocalStore, "LocalStore"),
    (SpuObservationComponent::ProgramCounter, "ProgramCounter"),
    (SpuObservationComponent::Channels, "Channels"),
    (SpuObservationComponent::Reservation, "Reservation"),
    (SpuObservationComponent::Outcome, "Outcome"),
    (SpuObservationComponent::Effects, "Effects"),
    (SpuObservationComponent::FaultDiscard, "FaultDiscard"),
    (SpuObservationComponent::Fpscr, "Fpscr"),
    (SpuObservationComponent::Signals, "Signals"),
    (SpuObservationComponent::Interrupts, "Interrupts"),
];

/// One stored start state that separates a row's sequence from its partner.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelationCounterexample {
    /// The fixture's name, unique within a store.
    pub name: String,
    /// The catalog row.
    pub relation: cellgov_spu::fuzz::SpuSequenceRelationId,
    /// The real register of each symbolic register.
    pub assignment: Vec<u8>,
    /// Every register the start state holds nonzero; the rest are zero.
    pub registers: BTreeMap<u8, [u8; 16]>,
    /// Every 16-byte local-store line the start state holds nonzero, by
    /// address; the rest are zero.
    pub local_store: BTreeMap<u32, [u8; 16]>,
    /// The first component, in comparison order, that differs.
    pub first_component: SpuObservationComponent,
}

/// A fixture that does not parse.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CounterexampleError {
    /// The text is not the fixture's JSON.
    #[error("counterexample JSON does not parse: {source}")]
    Json {
        /// The parser's refusal.
        #[source]
        source: serde_json::Error,
    },
    /// The fixture names a schema this build does not read.
    #[error("counterexample schema {found} is not {COUNTEREXAMPLE_SCHEMA_VERSION}")]
    Schema {
        /// The version the fixture names.
        found: u32,
    },
    /// No catalog row has the name.
    #[error("counterexample row {row} is not in the catalog")]
    UnknownRow {
        /// The name the fixture gives.
        row: String,
    },
    /// No observation component has the name.
    #[error("counterexample component {component} is not an observation component")]
    UnknownComponent {
        /// The name the fixture gives.
        component: String,
    },
    /// The assignment does not fit the row.
    #[error("counterexample {name} assigns {found} registers where its row names {expected}, or a register past the file")]
    Assignment {
        /// The fixture's name.
        name: String,
        /// The row's symbolic register count.
        expected: usize,
        /// The assignment's length.
        found: usize,
    },
    /// A register key or value is out of form.
    #[error("counterexample register {register} is not a register below 128 with 32 lowercase hex digits")]
    Register {
        /// The key as the fixture gives it.
        register: String,
    },
    /// Two fixtures in one store share a name.
    #[error("counterexample name {name} appears twice in the store")]
    DuplicateName {
        /// The shared name.
        name: String,
    },
    /// A local-store key or value is out of form.
    #[error("counterexample line {address} is not an aligned local-store line with 32 lowercase hex digits")]
    Line {
        /// The key as the fixture gives it.
        address: String,
    },
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct CounterexampleJson {
    schema_version: u32,
    name: String,
    row: String,
    assignment: Vec<u8>,
    registers: BTreeMap<String, String>,
    local_store: BTreeMap<String, String>,
    first_component: String,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoreJson {
    counterexamples: Vec<CounterexampleJson>,
}

fn hex(value: &[u8; 16]) -> String {
    format!("{:032x}", u128::from_be_bytes(*value))
}

fn parse_hex(text: &str) -> Option<[u8; 16]> {
    let lower = text.len() == 32
        && text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte));
    lower
        .then(|| u128::from_str_radix(text, 16).ok())
        .flatten()
        .map(u128::to_be_bytes)
}

fn row_name(relation: cellgov_spu::fuzz::SpuSequenceRelationId) -> String {
    format!("{relation:?}")
}

/// The catalog row named `name`.
pub(crate) fn row_by_name(name: &str) -> Option<&'static SpuSequenceRelation> {
    sequence_relations()
        .iter()
        .find(|row| row_name(row.id) == name)
}

fn component_name(component: SpuObservationComponent) -> &'static str {
    use SpuObservationComponent as C;
    match component {
        C::Registers => "Registers",
        C::LocalStore => "LocalStore",
        C::ProgramCounter => "ProgramCounter",
        C::Channels => "Channels",
        C::Reservation => "Reservation",
        C::Outcome => "Outcome",
        C::Effects => "Effects",
        C::FaultDiscard => "FaultDiscard",
        C::Fpscr => "Fpscr",
        C::Signals => "Signals",
        C::Interrupts => "Interrupts",
    }
}

impl RelationCounterexample {
    /// The fixture for `instance`, holding only its nonzero registers and
    /// local-store lines.
    pub(crate) fn from_instance(
        name: String,
        relation: &SpuSequenceRelation,
        instance: &RelationInstance,
        first_component: SpuObservationComponent,
    ) -> Self {
        let registers = (0..SPU_REG_COUNT)
            .filter(|&register| instance.start.regs[register] != [0; 16])
            .map(|register| (register as u8, instance.start.regs[register]))
            .collect();
        let local_store = instance
            .start
            .ls
            .chunks_exact(LINE)
            .enumerate()
            .filter(|(_, line)| line.iter().any(|&byte| byte != 0))
            .map(|(index, line)| {
                let mut value = [0u8; 16];
                value.copy_from_slice(line);
                ((index * LINE) as u32, value)
            })
            .collect();
        Self {
            name,
            relation: relation.id,
            assignment: instance.assignment.clone(),
            registers,
            local_store,
            first_component,
        }
    }

    /// The instance both sides start from.
    pub(crate) fn instance(&self) -> RelationInstance {
        let mut start = SpuState::new();
        for (&register, &value) in &self.registers {
            start.set_reg(usize::from(register), value);
        }
        for (&address, value) in &self.local_store {
            let at = address as usize;
            start.ls[at..at + LINE].copy_from_slice(value);
        }
        RelationInstance {
            assignment: self.assignment.clone(),
            start,
        }
    }

    /// The fixture as pretty-printed JSON.
    #[must_use]
    pub fn to_json(&self) -> String {
        let json = CounterexampleJson {
            schema_version: COUNTEREXAMPLE_SCHEMA_VERSION,
            name: self.name.clone(),
            row: row_name(self.relation),
            assignment: self.assignment.clone(),
            registers: self
                .registers
                .iter()
                .map(|(register, value)| (register.to_string(), hex(value)))
                .collect(),
            local_store: self
                .local_store
                .iter()
                .map(|(address, value)| (format!("0x{address:05x}"), hex(value)))
                .collect(),
            first_component: component_name(self.first_component).to_owned(),
        };
        // The fixture's fields are strings, integers and string maps, which
        // always serialize.
        let mut text = serde_json::to_string_pretty(&json).unwrap_or_default();
        text.push('\n');
        text
    }

    /// Parses one fixture.
    ///
    /// # Errors
    ///
    /// [`CounterexampleError`] names the first field out of form.
    #[cfg(test)]
    pub(crate) fn parse_json(text: &str) -> Result<Self, CounterexampleError> {
        let json: CounterexampleJson =
            serde_json::from_str(text).map_err(|source| CounterexampleError::Json { source })?;
        Self::from_json(json)
    }

    fn from_json(json: CounterexampleJson) -> Result<Self, CounterexampleError> {
        if json.schema_version != COUNTEREXAMPLE_SCHEMA_VERSION {
            return Err(CounterexampleError::Schema {
                found: json.schema_version,
            });
        }
        let row = row_by_name(&json.row).ok_or(CounterexampleError::UnknownRow {
            row: json.row.clone(),
        })?;
        let expected = row.register_count();
        if json.assignment.len() != expected
            || json
                .assignment
                .iter()
                .any(|&register| usize::from(register) >= SPU_REG_COUNT)
        {
            return Err(CounterexampleError::Assignment {
                name: json.name,
                expected,
                found: json.assignment.len(),
            });
        }
        let first_component = COMPONENTS
            .iter()
            .find(|(_, name)| *name == json.first_component)
            .map(|(component, _)| *component)
            .ok_or(CounterexampleError::UnknownComponent {
                component: json.first_component.clone(),
            })?;
        let mut registers = BTreeMap::new();
        for (key, value) in &json.registers {
            let register = key
                .parse::<u8>()
                .ok()
                .filter(|&register| usize::from(register) < SPU_REG_COUNT);
            match (register, parse_hex(value)) {
                (Some(register), Some(value)) => {
                    registers.insert(register, value);
                }
                _ => {
                    return Err(CounterexampleError::Register {
                        register: key.clone(),
                    })
                }
            }
        }
        let mut local_store = BTreeMap::new();
        for (key, value) in &json.local_store {
            let address = key
                .strip_prefix("0x")
                .and_then(|digits| u32::from_str_radix(digits, 16).ok())
                .filter(|&address| {
                    (address as usize).is_multiple_of(LINE) && (address as usize) < SPU_LS_SIZE
                });
            match (address, parse_hex(value)) {
                (Some(address), Some(value)) => {
                    local_store.insert(address, value);
                }
                _ => {
                    return Err(CounterexampleError::Line {
                        address: key.clone(),
                    })
                }
            }
        }
        Ok(Self {
            name: json.name,
            relation: row.id,
            assignment: json.assignment,
            registers,
            local_store,
            first_component,
        })
    }

    /// Runs both sides from the stored start state; the divergence when
    /// they still differ.
    ///
    /// # Errors
    ///
    /// [`FuzzError`] when the row no longer encodes or the catalog lost it.
    pub(crate) fn replay(&self) -> Result<Option<SequenceRelationDivergence>, FuzzError> {
        let relation = row_by_name(&row_name(self.relation)).ok_or(
            InvariantError::UnencodableSequenceRelation {
                relation: self.relation,
            },
        )?;
        Ok(match compare_relation(relation, &self.instance(), 0)? {
            RelationVerdict::Diverged(divergence) => Some(*divergence),
            RelationVerdict::Match | RelationVerdict::Inapplicable => None,
        })
    }
}

/// The path a campaign writes the fixture `name` to: `dir` with trailing
/// separators trimmed, one forward slash, then `<name>.json`.
#[must_use]
pub(crate) fn counterexample_path(dir: &str, name: &str) -> std::path::PathBuf {
    std::path::PathBuf::from(format!("{}/{name}.json", dir.trim_end_matches(['/', '\\'])))
}

/// Why a campaign stored no fixture for a relation counterexample.
#[derive(Debug, thiserror::Error)]
pub enum CounterexampleStoreError {
    /// The file system refused the directory, the write or the read-back.
    #[error("counterexample write {} failed: {source}", path.display())]
    Write {
        /// The fixture path.
        path: std::path::PathBuf,
        /// The file-system refusal.
        #[source]
        source: std::io::Error,
    },
    /// The path already holds a different fixture.
    #[error("counterexample path {} already holds a different fixture", path.display())]
    Collision {
        /// The fixture path.
        path: std::path::PathBuf,
    },
}

impl RelationCounterexample {
    /// Writes this fixture to `path`, creating the directory. The write is
    /// create-new; a file already there that holds the same fixture stands.
    ///
    /// # Errors
    ///
    /// [`CounterexampleStoreError`] for a write failure or a different
    /// fixture already at the path.
    pub(crate) fn store(&self, path: &std::path::Path) -> Result<(), CounterexampleStoreError> {
        use std::io::Write;
        let write = |source| CounterexampleStoreError::Write {
            path: path.to_path_buf(),
            source,
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(write)?;
        }
        let text = self.to_json();
        let created = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path);
        match created {
            Ok(mut file) => {
                file.write_all(text.as_bytes()).map_err(write)?;
                file.sync_all().map_err(write)
            }
            Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                let existing = std::fs::read_to_string(path).map_err(write)?;
                if existing == text {
                    Ok(())
                } else {
                    Err(CounterexampleStoreError::Collision {
                        path: path.to_path_buf(),
                    })
                }
            }
            Err(source) => Err(write(source)),
        }
    }
}

/// The counterexamples the committed store holds.
///
/// # Errors
///
/// [`CounterexampleError`] for the first stored fixture out of form.
pub(crate) fn stored_counterexamples() -> Result<Vec<RelationCounterexample>, CounterexampleError> {
    parse_store(STORE)
}

/// Parses a store: a JSON object whose `counterexamples` array holds
/// fixtures with distinct names.
pub(crate) fn parse_store(text: &str) -> Result<Vec<RelationCounterexample>, CounterexampleError> {
    let store: StoreJson =
        serde_json::from_str(text).map_err(|source| CounterexampleError::Json { source })?;
    let mut parsed: Vec<RelationCounterexample> = Vec::new();
    for json in store.counterexamples {
        let counterexample = RelationCounterexample::from_json(json)?;
        if parsed
            .iter()
            .any(|earlier| earlier.name == counterexample.name)
        {
            return Err(CounterexampleError::DuplicateName {
                name: counterexample.name,
            });
        }
        parsed.push(counterexample);
    }
    Ok(parsed)
}

/// Shrinks the start state of `instance` while the row still diverges in
/// `first_component`: registers outside the assignment to zero, then each
/// assigned register, then each of its words, then local store by halves.
///
/// [Regehr2012 p:3 s:3.2] A reducer keeps a smaller input only while it
/// still triggers the same failure. A pinned register keeps its value, since
/// every draw of its row holds the pin.
pub(crate) fn reduce_instance(
    relation: &SpuSequenceRelation,
    instance: &RelationInstance,
    first_component: SpuObservationComponent,
) -> Result<RelationInstance, FuzzError> {
    let mut reducer = Reducer {
        relation,
        first_component,
        budget: REDUCTION_BUDGET,
        best: instance.clone(),
    };
    let pinned: Vec<u8> = relation
        .pins
        .iter()
        .map(|&(symbolic, _)| instance.assignment[usize::from(symbolic)])
        .collect();
    let mut candidate = reducer.best.clone();
    for register in 0..SPU_REG_COUNT {
        if !instance.assignment.contains(&(register as u8)) {
            candidate.start.set_reg(register, [0; 16]);
        }
    }
    reducer.offer(candidate)?;
    let mut assigned = instance.assignment.clone();
    assigned.sort_unstable();
    assigned.dedup();
    for register in assigned.into_iter().filter(|r| !pinned.contains(r)) {
        let register = usize::from(register);
        let mut candidate = reducer.best.clone();
        candidate.start.set_reg(register, [0; 16]);
        if reducer.offer(candidate)? {
            continue;
        }
        for word in 0..4 {
            let mut value = reducer.best.start.regs[register];
            value[4 * word..4 * word + 4].fill(0);
            let mut candidate = reducer.best.clone();
            candidate.start.set_reg(register, value);
            reducer.offer(candidate)?;
        }
    }
    reducer.shrink_local_store(0, SPU_LS_SIZE)?;
    Ok(reducer.best)
}

struct Reducer<'a> {
    relation: &'a SpuSequenceRelation,
    first_component: SpuObservationComponent,
    budget: u32,
    best: RelationInstance,
}

impl Reducer<'_> {
    /// Keeps `candidate` when it differs from the best so far and still
    /// diverges in the same first component.
    fn offer(&mut self, candidate: RelationInstance) -> Result<bool, FuzzError> {
        if candidate == self.best || self.budget == 0 {
            return Ok(false);
        }
        self.budget -= 1;
        let keeps = matches!(
            compare_relation(self.relation, &candidate, 0)?,
            RelationVerdict::Diverged(divergence)
                if divergence.first_component == self.first_component
        );
        if keeps {
            self.best = candidate;
        }
        Ok(keeps)
    }

    /// Zeroes local store in `from..to`, or failing that in each half.
    fn shrink_local_store(&mut self, from: usize, to: usize) -> Result<(), FuzzError> {
        if self.best.start.ls[from..to].iter().all(|&byte| byte == 0) {
            return Ok(());
        }
        let mut candidate = self.best.clone();
        candidate.start.ls[from..to].fill(0);
        if self.offer(candidate)? || to - from <= LINE || self.budget == 0 {
            return Ok(());
        }
        let middle = from + (to - from) / 2;
        self.shrink_local_store(from, middle)?;
        self.shrink_local_store(middle, to)
    }
}

#[cfg(test)]
#[path = "tests/counterexample_tests.rs"]
mod tests;
