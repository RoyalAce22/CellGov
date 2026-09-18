//! CellGov's handling of every LV2 ordinal: the rows of `route.tsv` and `arm.tsv`.

use cellgov_ps3_abi::lv2::syscall::SYSCALL_TABLE_SLOTS;
use strum::VariantArray;

use super::spec::{ARM, ROUTE};
use super::table::{render, ArchiveError, NONE};
use crate::request::fidelity::{ArmFidelity, ROUTED_UNSUPPORTED_ARMS};
use crate::request::{classify, Lv2Request, Lv2RequestKind, RUNTIME_FAST_PATH};

/// The path a syscall number takes at dispatch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// Classifies to a typed arm.
    Typed,
    /// Reaches a dedicated arm inside `Unsupported`.
    Routed,
    /// The honest traced `CELL_ENOSYS` refusal.
    NullBackend,
    /// The runtime's timer path answers it before classification.
    RuntimeFastPath,
}

impl Route {
    /// Every route, in the order the archive document lists them.
    pub const ALL: &[Route] = &[
        Route::Typed,
        Route::Routed,
        Route::NullBackend,
        Route::RuntimeFastPath,
    ];

    /// The stable label `route.tsv` carries.
    pub fn label(self) -> &'static str {
        match self {
            Route::Typed => "typed",
            Route::Routed => "routed",
            Route::NullBackend => "null_backend",
            Route::RuntimeFastPath => "runtime_fast_path",
        }
    }

    /// One-line meaning of the route, as the archive document states it.
    pub fn meaning(self) -> &'static str {
        match self {
            Route::Typed => "Classifies to a typed request arm.",
            Route::Routed => "Reaches a dedicated arm inside `Unsupported`.",
            Route::NullBackend => {
                "The honest traced `CELL_ENOSYS` refusal through the generic arm."
            }
            Route::RuntimeFastPath => {
                "Answered by the runtime's timer path; never classified or dispatched."
            }
        }
    }
}

/// One row of `route.tsv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteRow {
    /// The syscall number.
    pub ordinal: u64,
    /// The path the number takes.
    pub route: Route,
    /// The arm it reaches, for a typed or routed number.
    pub arm: Option<&'static str>,
}

/// One row of `arm.tsv`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmRow {
    /// The arm identifier: a typed variant name or a routed arm name.
    pub arm: &'static str,
    /// The reviewed fidelity tag.
    pub fidelity: ArmFidelity,
    /// The ordinals that reach the arm, ascending.
    pub ordinals: Vec<u64>,
}

/// How many slots take each route.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct HandlingCounts {
    /// Slots that classify to a typed arm.
    pub typed: usize,
    /// Slots that reach a dedicated `Unsupported` arm.
    pub routed: usize,
    /// Slots the null backend refuses.
    pub null_backend: usize,
    /// Slots the runtime answers on its timer path.
    pub runtime_fast_path: usize,
}

impl HandlingCounts {
    /// Tally `routes` by route.
    pub fn of(routes: &[RouteRow]) -> Self {
        let mut counts = Self::default();
        for row in routes {
            let slot = match row.route {
                Route::Typed => &mut counts.typed,
                Route::Routed => &mut counts.routed,
                Route::NullBackend => &mut counts.null_backend,
                Route::RuntimeFastPath => &mut counts.runtime_fast_path,
            };
            *slot += 1;
        }
        counts
    }

    /// The tally for one route.
    pub fn of_route(&self, route: Route) -> usize {
        match route {
            Route::Typed => self.typed,
            Route::Routed => self.routed,
            Route::NullBackend => self.null_backend,
            Route::RuntimeFastPath => self.runtime_fast_path,
        }
    }
}

/// One row per LV2 slot, from the zero-argument probe.
///
/// # Panics
///
/// Panics if the zero-argument probe yields one of these rejections:
///
/// - `NoSuchSyscall`
/// - `Malformed`
/// - `Hypercall`
/// - `UnresolvedImport`
///
/// Such a rejection would reduce the apparent typed-slot count.
pub fn route_rows() -> Vec<RouteRow> {
    (0..SYSCALL_TABLE_SLOTS)
        .map(|ordinal| {
            if RUNTIME_FAST_PATH.contains(&ordinal) {
                return RouteRow {
                    ordinal,
                    route: Route::RuntimeFastPath,
                    arm: None,
                };
            }
            match classify(ordinal, &[0u64; 8]) {
                Lv2Request::Unsupported { .. } => {
                    match ROUTED_UNSUPPORTED_ARMS.iter().find(|(n, ..)| *n == ordinal) {
                        Some((_, arm, _)) => RouteRow {
                            ordinal,
                            route: Route::Routed,
                            arm: Some(arm),
                        },
                        None => RouteRow {
                            ordinal,
                            route: Route::NullBackend,
                            arm: None,
                        },
                    }
                }
                Lv2Request::NoSuchSyscall { .. }
                | Lv2Request::Malformed { .. }
                | Lv2Request::Hypercall { .. }
                | Lv2Request::UnresolvedImport { .. } => {
                    panic!("slot {ordinal}: zero probe args classified as a rejection; the census is invalid")
                }
                typed => RouteRow {
                    ordinal,
                    route: Route::Typed,
                    arm: Some(Lv2RequestKind::from(&typed).into()),
                },
            }
        })
        .collect()
}

/// One row per arm, sorted by arm name, with the slots of `routes` that reach it.
///
/// The arms are:
///
/// - every typed variant that carries a fidelity tag
/// - every routed arm
pub fn arm_rows(routes: &[RouteRow]) -> Vec<ArmRow> {
    let served = |arm: &str| -> Vec<u64> {
        routes
            .iter()
            .filter(|row| row.arm == Some(arm))
            .map(|row| row.ordinal)
            .collect()
    };
    let mut rows: Vec<ArmRow> = Lv2RequestKind::VARIANTS
        .iter()
        .filter_map(|kind| {
            let arm: &'static str = (*kind).into();
            kind.fidelity().map(|fidelity| ArmRow {
                arm,
                fidelity,
                ordinals: served(arm),
            })
        })
        .collect();
    rows.extend(
        ROUTED_UNSUPPORTED_ARMS
            .iter()
            .map(|(_, arm, fidelity)| ArmRow {
                arm,
                fidelity: *fidelity,
                ordinals: served(arm),
            }),
    );
    rows.sort_by(|a, b| a.arm.cmp(b.arm));
    rows
}

/// The `route.tsv` text for `routes`.
///
/// # Errors
///
/// Whatever [`render`] refuses in the rendered rows.
pub fn route_tsv(routes: &[RouteRow]) -> Result<String, ArchiveError> {
    let rows: Vec<Vec<String>> = routes
        .iter()
        .map(|row| {
            vec![
                row.ordinal.to_string(),
                row.route.label().to_string(),
                row.arm.unwrap_or(NONE).to_string(),
            ]
        })
        .collect();
    render(&ROUTE, &rows)
}

/// The `arm.tsv` text for `arms`.
///
/// # Errors
///
/// Whatever [`render`] refuses in the rendered rows.
pub fn arm_tsv(arms: &[ArmRow]) -> Result<String, ArchiveError> {
    let rows: Vec<Vec<String>> = arms
        .iter()
        .map(|row| {
            let ordinals = if row.ordinals.is_empty() {
                NONE.to_string()
            } else {
                let items: Vec<String> = row.ordinals.iter().map(u64::to_string).collect();
                items.join(",")
            };
            vec![
                row.arm.to_string(),
                row.fidelity.label().to_string(),
                ordinals,
            ]
        })
        .collect();
    render(&ARM, &rows)
}

#[cfg(test)]
#[path = "tests/handling_tests.rs"]
mod tests;
