//! The loader fuzz targets: one call per parser that takes
//! attacker-shaped bytes, and a bounded mutation sweep that runs the
//! same calls without a coverage-guided fuzzer.
//!
//! The property every target checks is the same: the parser returns,
//! with a value or its own typed error, on any input. A panic is the
//! finding. The `cargo fuzz` targets under `fuzz/` call [`exercise`]
//! and let the fuzzer catch the panic. [`sweep`] catches it at the
//! target boundary, so the continuous build can run a bounded sample
//! of the same property on a stable toolchain.

use cellgov_mem::GuestMemory;
use cellgov_ppu::funcmap::FuncMapError;
use cellgov_ppu::loader::LoadError;
use cellgov_ppu::prx::ImportParseError;
use cellgov_ppu::sprx::PrxParseError;
use cellgov_ppu::state::PpuState;

use crate::boundary::call_target;
use crate::loader_images::structured_image;
use crate::rng::Rng;
use crate::{GeneratorError, TargetPanicPayload, CAMPAIGN_VERSION};

/// Guest memory a `load_ppu_elf` case loads into. The loader refuses a
/// segment past it as out of range, which is the typed path the target
/// wants.
const LOAD_MEMORY_BYTES: usize = 1 << 20;
/// Longest input the sweep keeps after mutation.
const MAX_INPUT_BYTES: usize = 1 << 16;
/// Most bytes one insertion adds.
const MAX_INSERT_BYTES: u64 = 16;
/// Mutation rounds one case applies, at most.
const MAX_MUTATION_ROUNDS: u64 = 4;

/// One parser the fuzz targets cover.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum LoaderTarget {
    /// `cellgov_ppu::loader::pt_load_segments`.
    PtLoadSegments,
    /// `cellgov_ppu::loader::load_ppu_elf` into a bounded guest memory.
    LoadPpuElf,
    /// `cellgov_ppu::sprx::parse_prx`.
    ParsePrx,
    /// `cellgov_ppu::prx::parse_imports`.
    ParseImports,
    /// `cellgov_ppu::funcmap::build`.
    FuncmapBuild,
}

impl LoaderTarget {
    /// Every target, in the order `fuzz/Cargo.toml` and the workflow list them.
    pub const ALL: [Self; 5] = [
        Self::PtLoadSegments,
        Self::LoadPpuElf,
        Self::ParsePrx,
        Self::ParseImports,
        Self::FuncmapBuild,
    ];

    /// The name the `fuzz/fuzz_targets/<name>.rs` file and the workflow
    /// matrix use.
    pub fn name(self) -> &'static str {
        match self {
            Self::PtLoadSegments => "pt_load_segments",
            Self::LoadPpuElf => "load_ppu_elf",
            Self::ParsePrx => "parse_prx",
            Self::ParseImports => "parse_imports",
            Self::FuncmapBuild => "funcmap_build",
        }
    }

    /// The target `name` names, if any.
    pub fn parse_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|target| target.name() == name)
    }
}

impl std::fmt::Display for LoaderTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// The typed error a parser returned.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoaderRefusal {
    /// The ELF loader's refusal.
    #[error(transparent)]
    Elf(#[from] LoadError),
    /// The PRX parser's refusal.
    #[error(transparent)]
    Prx(#[from] PrxParseError),
    /// The import walk's refusal.
    #[error(transparent)]
    Imports(#[from] ImportParseError),
    /// The function-map builder's refusal.
    #[error(transparent)]
    FuncMap(#[from] FuncMapError),
}

/// What one parser call returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoaderOutcome {
    /// The parser produced a value.
    Accepted,
    /// The parser returned its typed error.
    Refused(LoaderRefusal),
}

/// Run `target` on `data` once. Panics propagate: that is the finding.
pub fn run(target: LoaderTarget, data: &[u8]) -> LoaderOutcome {
    let refusal = match target {
        LoaderTarget::PtLoadSegments => cellgov_ppu::loader::pt_load_segments(data)
            .map(drop)
            .map_err(LoaderRefusal::Elf),
        LoaderTarget::LoadPpuElf => {
            let mut memory = GuestMemory::new(LOAD_MEMORY_BYTES);
            let mut state = PpuState::new();
            cellgov_ppu::loader::load_ppu_elf(data, &mut memory, &mut state)
                .map(drop)
                .map_err(LoaderRefusal::Elf)
        }
        LoaderTarget::ParsePrx => cellgov_ppu::sprx::parse_prx(data)
            .map(drop)
            .map_err(LoaderRefusal::Prx),
        LoaderTarget::ParseImports => cellgov_ppu::prx::parse_imports(data)
            .map(drop)
            .map_err(LoaderRefusal::Imports),
        LoaderTarget::FuncmapBuild => cellgov_ppu::funcmap::build(data)
            .map(drop)
            .map_err(LoaderRefusal::FuncMap),
    };
    match refusal {
        Ok(()) => LoaderOutcome::Accepted,
        Err(refusal) => LoaderOutcome::Refused(refusal),
    }
}

/// Which bytes a case handed the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputPath {
    /// The fuzz bytes themselves.
    Raw,
    /// The image the fuzz bytes describe, see
    /// [`structured_image`].
    Structured,
}

/// Both parser calls one fuzz input drives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseOutcome {
    /// The call on the raw bytes.
    pub raw: LoaderOutcome,
    /// The call on the structured image.
    pub structured: LoaderOutcome,
}

/// Run `target` on the raw bytes and on the image they describe. This
/// is the body of every `cargo fuzz` target; a panic propagates.
pub fn exercise(target: LoaderTarget, data: &[u8]) -> CaseOutcome {
    let raw = run(target, data);
    let structured = run(target, &structured_image(data));
    CaseOutcome { raw, structured }
}

/// A parser panic the sweep caught, with the input that replays it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderPanic {
    /// The target that panicked.
    pub target: LoaderTarget,
    /// Which call panicked.
    pub path: InputPath,
    /// The fuzz bytes of the case.
    pub input: Vec<u8>,
    /// The panic message, classified.
    pub payload: TargetPanicPayload,
}

/// How much a sweep runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepConfig {
    /// Master seed of the mutations.
    pub seed: u64,
    /// Cases to run.
    pub cases: u64,
    /// Panics to keep; the sweep counts later ones and does not store them.
    pub max_panics: usize,
}

/// What a sweep found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepReport {
    /// The target swept.
    pub target: LoaderTarget,
    /// Cases run.
    pub cases: u64,
    /// Parser calls that produced a value, over both paths.
    pub accepted: u64,
    /// Parser calls that returned their typed error, over both paths.
    pub refused: u64,
    /// Parser calls that panicked, over both paths.
    pub panicked: u64,
    /// The first panics, up to [`SweepConfig::max_panics`].
    pub panics: Vec<LoaderPanic>,
}

/// Run `config.cases` mutations of `seeds` through `target` and catch
/// every panic at the target boundary.
///
/// # Errors
///
/// [`GeneratorError`] when the mutation draws fail, which an empty
/// `seeds` causes.
pub fn sweep(
    target: LoaderTarget,
    seeds: &[Vec<u8>],
    config: SweepConfig,
) -> Result<SweepReport, GeneratorError> {
    let mut report = SweepReport {
        target,
        cases: 0,
        accepted: 0,
        refused: 0,
        panicked: 0,
        panics: Vec::new(),
    };
    for case in 0..config.cases {
        let mut rng = Rng::for_case(CAMPAIGN_VERSION, config.seed, case);
        // `below` draws under the seed count, which is a `usize`.
        let base = &seeds[rng.below(seeds.len() as u64)? as usize];
        let input = mutate(&mut rng, base)?;
        // The image renders outside the target boundary, so a renderer
        // panic fails the sweep itself.
        let image = structured_image(&input);
        for path in [InputPath::Raw, InputPath::Structured] {
            let outcome = match path {
                InputPath::Raw => call_target(|| run(target, &input)),
                InputPath::Structured => call_target(|| run(target, &image)),
            };
            match outcome {
                Ok(LoaderOutcome::Accepted) => report.accepted += 1,
                Ok(LoaderOutcome::Refused(_)) => report.refused += 1,
                Err(payload) => {
                    report.panicked += 1;
                    if report.panics.len() < config.max_panics {
                        report.panics.push(LoaderPanic {
                            target,
                            path,
                            input: input.clone(),
                            payload,
                        });
                    }
                }
            }
        }
        report.cases += 1;
    }
    Ok(report)
}

/// One to four byte-level mutations of `base`.
fn mutate(rng: &mut Rng, base: &[u8]) -> Result<Vec<u8>, GeneratorError> {
    let mut out = base.to_vec();
    let rounds = 1 + rng.below(MAX_MUTATION_ROUNDS)?;
    for _ in 0..rounds {
        if out.is_empty() {
            out.push(rng.next_u32() as u8);
            continue;
        }
        let len = out.len();
        let pos = rng.below(len as u64)? as usize;
        match rng.below(7)? {
            0 => out[pos] ^= 1 << rng.below(8)?,
            1 => out[pos] = [0, 0xFF, 0x7F, 0x80][rng.below(4)? as usize],
            2 => {
                let at = (pos & !3).min(len.saturating_sub(4));
                if at + 4 <= len {
                    let word = rng.mixed_u64()? as u32;
                    out[at..at + 4].copy_from_slice(&word.to_be_bytes());
                }
            }
            3 => {
                let at = (pos & !7).min(len.saturating_sub(8));
                if at + 8 <= len {
                    out[at..at + 8].copy_from_slice(&rng.mixed_u64()?.to_be_bytes());
                }
            }
            4 => out.truncate(pos),
            5 => {
                let count = rng.below(MAX_INSERT_BYTES + 1)? as usize;
                let mut insert = vec![0u8; count];
                rng.fill(&mut insert);
                out.splice(pos..pos, insert);
            }
            _ => {
                let end = (pos + 1 + rng.below(64)? as usize).min(len);
                let chunk = out[pos..end].to_vec();
                out.splice(pos..pos, chunk);
            }
        }
    }
    out.truncate(MAX_INPUT_BYTES);
    Ok(out)
}

#[cfg(test)]
#[path = "tests/loaders_tests.rs"]
mod tests;
