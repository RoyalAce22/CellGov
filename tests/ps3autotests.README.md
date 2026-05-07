# ps3autotests fixture

The [ps3autotests](https://github.com/AerialX/ps3autotests) test
corpus -- small `.ppu.elf` programs and the TTY output captured from
a real PS3 -- backs
[`apps/cellgov_cli/tests/ps3autotests.rs`](../apps/cellgov_cli/tests/ps3autotests.rs).
The harness reboots each whitelisted ELF and asserts that CellGov's
HLE syscalls produce byte-identical TTY to real hardware.

The corpus is GPLv2 and is **not** vendored: vendoring would force
CellGov itself onto GPLv2. Clone it yourself into the gitignored
`tests/ps3autotests/` directory:

```bash
git clone https://github.com/AerialX/ps3autotests.git tests/ps3autotests
```

After cloning, `tests/ps3autotests/tests/cpu/basic/basic.ppu.elf` and
the other whitelisted ELFs exist; `cargo test -p cellgov_cli --test
ps3autotests` exercises them. Without the corpus the tests skip
silently so CI without the fixture stays green.

## Force-require the fixture

For release-gate runners that should fail rather than skip:

```bash
CELLGOV_REQUIRE_AUTOTESTS=1 cargo test -p cellgov_cli --test ps3autotests
```

## Line-ending caveat

The `.expected` files contain raw TTY bytes; `\n` newlines are
significant. The repo's `.gitattributes` carries
`tests/ps3autotests/**/*.expected -text` to block autocrlf rewriting
in the unlikely case anything in there ever gets tracked, but it does
not affect your local clone of the upstream corpus. If your global
`core.autocrlf=true` mangled the corpus on clone, run
`git -C tests/ps3autotests config core.autocrlf input` and
re-checkout. The harness detects `\r`-count mismatches at runtime and
prints a hint when one shows up.
