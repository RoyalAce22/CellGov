# Loader fuzz targets

`cargo fuzz` targets for the parsers that take attacker-shaped input:

| Target             | Parser                                   |
| ------------------ | ---------------------------------------- |
| `pt_load_segments` | `cellgov_ppu::loader::pt_load_segments`  |
| `load_ppu_elf`     | `cellgov_ppu::loader::load_ppu_elf`      |
| `parse_prx`        | `cellgov_ppu::sprx::parse_prx`           |
| `parse_imports`    | `cellgov_ppu::prx::parse_imports`        |
| `funcmap_build`    | `cellgov_ppu::funcmap::build`            |

Every target asserts one property: the parser returns, with a value or
its own typed error, on any bytes. A panic is the finding. Each input
drives two calls, one on the raw bytes and one on the image the bytes
describe when read as fields of an ELF or PRX
(`cellgov_fuzz::loader_images::structured_image`), so the fuzzer
reaches the table walks and relocation arithmetic instead of stopping
at the magic check.

This package is not a workspace member. libFuzzer needs a nightly
toolchain, and the workspace builds on the pinned stable; the
`.github/workflows/loader-fuzz.yml` workflow runs the targets on a
schedule. A bounded sample of the same property runs on stable inside
`cargo test -p cellgov_fuzz` (`loaders::tests`), so a panic a seed
mutation reaches fails the continuous build without libFuzzer.

## Running locally

```bash
rustup toolchain install nightly
cargo install cargo-fuzz --version 0.13.2 --locked

# Seed the corpus and fuzz one target for fifteen minutes.
bash .github/loader_fuzz.sh run parse_prx

# Shorter, or a different nightly.
FUZZ_SECONDS=60 FUZZ_TOOLCHAIN=nightly-2026-08-22 bash .github/loader_fuzz.sh run funcmap_build
```

On a Windows host the binaries link against the MSVC AddressSanitizer
runtime and refuse to start until its directory is on the path:

```bash
PATH="/c/Program Files/Microsoft Visual Studio/2022/Community/VC/Tools/MSVC/<version>/bin/Hostx64/x64:$PATH" \
  bash .github/loader_fuzz.sh run parse_prx
```

Everything a run writes lives under `fuzz/corpus/<target>/` and
`fuzz/artifacts/<target>/`, both ignored by git. The workflow caches
each target's corpus between runs.

## Seeds

`fuzz/seeds/*.bin` are the committed starting images, rendered from
`cellgov_fuzz::loader_images::seeds`: minimal executables, a module
with exports and system OPDs, modules with import tables located both
ways, zero-sized placeholder segments. `loaders::tests` fails when the
files drift from the generator; after changing a seed image, run

```bash
cargo test -p cellgov_fuzz --lib -- --ignored regenerate_seed_corpus
```

Real images make better seeds than synthetic ones. Copy any decrypted
firmware module or title executable into `fuzz/corpus/<target>/` before
a run; `cellgov self decrypt` unwraps an SCE-wrapped one. They are
never committed.

## A finding

libFuzzer writes the input that crashed to
`fuzz/artifacts/<target>/crash-<hash>` and the workflow uploads that
directory. Replay it with

```bash
cargo +nightly fuzz run --fuzz-dir fuzz <target> fuzz/artifacts/<target>/crash-<hash>
```

then pin the minimised input as a named unit test beside the parser in
`cellgov_ppu` and fix the parser to return its typed error. A file that
merely fails to parse is not a finding; the targets accept every
refusal.
