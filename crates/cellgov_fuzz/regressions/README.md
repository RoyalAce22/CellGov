# Promoted fuzz findings

Every real finding the fuzzer reports is promoted here before its fix
lands: the minimized artifact `cellgov dev fuzz smoke` stored for it is
copied to `<name>.json`, and `manifest.json` lists the name as `open`
with the build profile it reproduces in and a one-sentence summary. The
fix flips the entry to `fixed` in the same change. A test replays every
entry on every build: an open entry must reproduce, a fixed entry must
not, a file no entry names refuses the directory, and so does an entry
with no file or an artifact that was never reduced.

A finding the smoke set reports that no open entry covers fails the
build. Promote it or fix it; nothing here is skipped.
