use cellgov_mem::scheme::{
    HashedSurface, Stamp, CHECKPOINT_VERSION, FIRST_STAMPED_VERSION, RUNTIME_SURFACES,
};

use super::*;

/// A rule the hashed surfaces break against the checkpoint tag.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SchemeFault {
    /// A surface's stamp is newer than the tag: the tag did not move.
    AboveTag { name: &'static str, version: u32 },
    /// A surface's stamp is older than the first stamped version.
    BeforeFirst { name: &'static str, version: u32 },
    /// No surface has this version as its stamp: the tag moved with no
    /// surface to cover.
    Unstamped { version: u32 },
    /// A surface is still pending in a strict run.
    PendingInStrictRun { name: &'static str },
}

/// Whether the run refuses a pending surface, which
/// `CELLGOV_SCHEME_STRICT=1` asks for: the run that moves the tag.
fn strict() -> bool {
    std::env::var_os("CELLGOV_SCHEME_STRICT").is_some_and(|v| v == "1")
}

/// The rules `surfaces` break against the tag version `tag`, where
/// `first` is the first stamped version: each surface's faults in order,
/// then each unstamped version, ascending. `strict` refuses a pending
/// surface.
fn scheme_faults(
    tag: u32,
    first: u32,
    surfaces: &[HashedSurface],
    strict: bool,
) -> Vec<SchemeFault> {
    let mut faults = Vec::new();
    for surface in surfaces {
        match surface.stamp {
            Stamp::Version(version) if version > tag => faults.push(SchemeFault::AboveTag {
                name: surface.name,
                version,
            }),
            Stamp::Version(version) if version < first => {
                faults.push(SchemeFault::BeforeFirst {
                    name: surface.name,
                    version,
                });
            }
            Stamp::Pending if strict => {
                faults.push(SchemeFault::PendingInStrictRun { name: surface.name })
            }
            Stamp::Version(_) | Stamp::Pending => {}
        }
    }
    for version in first..=tag {
        if !surfaces
            .iter()
            .any(|surface| surface.stamp == Stamp::Version(version))
        {
            faults.push(SchemeFault::Unstamped { version });
        }
    }
    faults
}

const fn surface(name: &'static str, stamp: Stamp) -> HashedSurface {
    HashedSurface {
        name,
        source: None,
        stamp,
    }
}

const fn v(version: u32) -> Stamp {
    Stamp::Version(version)
}

/// A case: what it shows, the tag version, the surfaces, whether the run
/// is strict, and the faults.
type Case<'a> = (&'a str, u32, &'a [HashedSurface], bool, &'a [SchemeFault]);

#[test]
fn the_scheme_guard_refuses_each_broken_rule() {
    let cases: &[Case<'_>] = &[
        (
            "each version from the first to the tag is a stamp",
            24,
            &[
                surface("a", v(22)),
                surface("b", v(23)),
                surface("c", v(24)),
            ],
            false,
            &[],
        ),
        (
            "a pending surface in a run that is not strict",
            23,
            &[
                surface("a", v(22)),
                surface("b", v(23)),
                surface("c", Stamp::Pending),
            ],
            false,
            &[],
        ),
        (
            "a pending surface in a strict run",
            23,
            &[
                surface("a", v(22)),
                surface("b", v(23)),
                surface("c", Stamp::Pending),
            ],
            true,
            &[SchemeFault::PendingInStrictRun { name: "c" }],
        ),
        (
            "a stamp the tag has not reached",
            23,
            &[
                surface("a", v(22)),
                surface("b", v(23)),
                surface("c", v(24)),
            ],
            false,
            &[SchemeFault::AboveTag {
                name: "c",
                version: 24,
            }],
        ),
        (
            "a tag that moved with no surface to cover",
            24,
            &[surface("a", v(22)), surface("b", v(23))],
            false,
            &[SchemeFault::Unstamped { version: 24 }],
        ),
        (
            "a version between two stamps that no surface has",
            24,
            &[surface("a", v(22)), surface("b", v(24))],
            false,
            &[SchemeFault::Unstamped { version: 23 }],
        ),
        (
            "a stamp older than the first stamped version",
            23,
            &[
                surface("a", v(21)),
                surface("b", v(22)),
                surface("c", v(23)),
            ],
            false,
            &[SchemeFault::BeforeFirst {
                name: "a",
                version: 21,
            }],
        ),
    ];
    assert_eq!(cases.len(), 7);
    for (what, tag, surfaces, strict, expected) in cases {
        assert_eq!(
            scheme_faults(*tag, 22, surfaces, *strict),
            *expected,
            "{what}"
        );
    }
}

#[test]
fn the_checkpoint_tag_is_the_newest_stamp_of_the_hashed_surfaces() {
    let surfaces: Vec<HashedSurface> = RUNTIME_SURFACES
        .iter()
        .chain(GROUPS.iter().flat_map(|group| group.hashed_surfaces))
        .copied()
        .collect();
    assert!(!surfaces.is_empty(), "no hashed surface is declared");
    assert_eq!(
        scheme_faults(
            CHECKPOINT_VERSION,
            FIRST_STAMPED_VERSION,
            &surfaces,
            strict()
        ),
        []
    );
}
