# ps3autotests fixture

This directory holds the [ps3autotests](https://github.com/AerialX/ps3autotests)
test corpus -- a collection of small `.ppu.elf` programs and the TTY
output captured from a real PS3, used by
[`apps/cellgov_cli/tests/ps3autotests.rs`](../../apps/cellgov_cli/tests/ps3autotests.rs)
to validate that CellGov's HLE syscalls produce byte-identical TTY
output to real hardware.

The corpus is licensed GPLv2 and is **not** vendored into this repo,
which would force CellGov itself onto GPLv2. Clone it yourself:

```bash
git clone https://github.com/AerialX/ps3autotests.git tests/ps3autotests
```

After cloning, `tests/ps3autotests/tests/cpu/basic/basic.ppu.elf` (and
the other whitelisted ELFs in `apps/cellgov_cli/tests/ps3autotests.rs`)
should exist. Running `cargo test -p cellgov_cli --test ps3autotests`
will then exercise them. Without the fixture present the tests log a
skip note and return clean, so CI without the corpus stays green.

## Forcing the fixture to be present

To make a missing corpus a hard failure (e.g. on a release-gate runner)
set `CELLGOV_REQUIRE_AUTOTESTS` to any non-empty value:

```bash
CELLGOV_REQUIRE_AUTOTESTS=1 cargo test -p cellgov_cli --test ps3autotests
```

## Line-ending caveat

The `.expected` files contain raw TTY bytes; their `\n` newlines are
significant. The repo-level `.gitattributes` declares
`tests/ps3autotests/**/*.expected -text` so that any future vendoring
or vendoring-by-mistake cannot silently rewrite them to `\r\n` under
Windows autocrlf. If your local clone of the upstream corpus already
has CRLF mangling (because `core.autocrlf=true` was set globally when
you cloned), run `git -C tests/ps3autotests config core.autocrlf input`
and re-checkout. The harness also detects CR-count mismatches at
runtime and prints a hint when it spots one.
