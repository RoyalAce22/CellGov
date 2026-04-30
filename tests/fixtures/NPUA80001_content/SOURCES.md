# NPUA80001 (flOw) content fixtures

Synthetic minimal XMLs that satisfy `sys_fs_open` lookups for the
title's resource loader when no real content is available. The
boot-time content provider in
`apps/cellgov_cli/src/game/content.rs` reads each file's bytes and
registers them in `Lv2Host::fs_store` at the manifest-named guest
path. Missing host files surface as a startup error (loud failure)
rather than runtime ENOENT (silent divergence).

## Real-content resolution (three tiers)

The boot-time content provider resolves each manifest entry against
ONE of three bases, in priority order:

1. **Override env var** (`CELLGOV_NPUA80001_CONTENT_DIR` for flOw,
   per `override_base_env` in the manifest). Explicit; missing
   files under it are a hard startup error that names the env var.
2. **EBOOT-adjacent USRDIR** (auto-discovered from the EBOOT
   path's parent dir). Soft probe: the tier is used only when
   every manifest entry resolves under it; a partial USRDIR falls
   through to (3). This is the natural default for any developer
   running CellGov against their PSN install -- the real flOw XMLs
   already live at
   `USRDIR/Data/{Resources,Local,Classes}/*.xml` next to
   `EBOOT.elf`.
3. **Manifest's checked-in `base`** (the synthetic stubs here).
   Used when neither (1) nor (2) is available. Hard-fail on
   missing files.

The boot banner echoes which tier was used so a developer can
confirm.

## flOw's content distribution

flOw is a digital-only PSN download (no DLC); every XML the
resource loader needs ships inside the title's `USRDIR/Data/`
tree alongside `EBOOT.elf` on every PSN install. A developer with
the title installed locally needs no setup -- as long as the
EBOOT loads from a real PSN install layout, USRDIR auto-discovery
picks up the real bytes. The override env var is the
explicit-redirect path for developers who want to point at a
different content tree (e.g., a stripped or modded variant).

## Synthetic stubs (this directory)

The XMLs at:

- `Data/Resources/first.xml`
- `Data/Local/Localization.xml`
- `Data/Classes/Classes.xml`

are hand-authored stubs with no copyrighted content. They satisfy
`sys_fs_open` / `sys_fs_read` / `sys_fs_fstat` so the title's
resource loader can attempt to parse them, but they do NOT satisfy
the title's XML schema validator -- the title still trips
`MODE_AUTO_LOAD` on the parse-fail branch. The structural fix to
get past that is real content via tier (1) or (2); the synthetic
stubs exist so the public test suite has something to register
without external dependencies, and so the schema + provider
plumbing is exercised end-to-end in CI.
