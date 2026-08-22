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
should exist.

## Running the suite

The suite sits behind the `ps3autotests` cargo feature, so a checkout
without the corpus does not build it and CI stays green without
skipping anything:

```bash
cargo test -p cellgov_cli --features ps3autotests --test ps3autotests
```

Enabling the feature declares the corpus present: a missing clone is a
hard failure naming the path, never a silent pass. There is no env var
to set.

Every case is additionally `#[ignore]`d on an unrelated blocker -- the
synthetic ELFs import `sysPrxForUser` NIDs that no HLE module binds --
so they report as ignored until that is resolved. Each test's ignore
reason states the condition.

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
