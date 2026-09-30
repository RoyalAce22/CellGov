#!/bin/bash
# Build the spu_float_edges microtest.
#
# spu/cases.S and cases.h are generated from cases.tsv by the ignored
# `regenerate` test in crates/cellgov_spu/tests/it/spu_float_edges.rs; run it
# after editing cases.tsv, then build.
#
# Requirements -- any environment providing:
#   1. the ps3dev toolchains (powerpc64-ps3-elf-gcc and spu-gcc) and
#      PSL1GHT, installed under $PS3DEV / $PSL1GHT (default
#      /usr/local/ps3dev; override via env),
#   2. python3 (for common/patch_toc.py),
#   3. this test directory mounted at /src and the shared
#      tests/micro/common at /common (or as /src/../common).
#
# The pinned toolchain is built from tests/micro/toolchain/Dockerfile:
#
#   docker build -t ps3dev tests/micro/toolchain
#   docker run --rm -v /path/to/spu_float_edges:/src \
#       -v /path/to/common:/common \
#       -e COMMON=/common ps3dev bash /src/build.sh
#
# Git Bash on Windows rewrites the /src and /common mount targets;
# prefix the command with MSYS_NO_PATHCONV=1 there.
#
set -e

PS3DEV="${PS3DEV:-/usr/local/ps3dev}"
PSL1GHT="${PSL1GHT:-$PS3DEV}"
SPU_PREFIX="spu"
PPU_PREFIX="powerpc64-ps3-elf"

OUT=/src/build
mkdir -p "$OUT"

echo "=== Building SPU program ==="
${SPU_PREFIX}-gcc \
    -I${PSL1GHT}/spu/include \
    -L${PSL1GHT}/spu/lib \
    -O2 -Wall \
    -o "$OUT/spu_main.elf" \
    /src/spu/main.c /src/spu/cases.S \
    -lsputhread

COMMON="${COMMON:-/src/../common}"

echo "=== Assembling custom CRT0 ==="
${PPU_PREFIX}-gcc \
    -c -o "$OUT/crt0.o" \
    "$COMMON/crt0.S"

echo "=== Linking PPU program ==="
${PPU_PREFIX}-gcc \
    -nostartfiles \
    -I${PSL1GHT}/ppu/include \
    -L${PSL1GHT}/ppu/lib \
    -O2 -Wall \
    -o "$OUT/spu_float_edges.elf" \
    "$OUT/crt0.o" \
    /src/ppu/main.c \
    -llv2 -lsysmodule -lrt

echo "=== Patching TOC and rldicr ==="
python3 "$COMMON/patch_toc.py" \
    "$OUT/spu_float_edges.elf" \
    "${PPU_PREFIX}-readelf" \
    "${PPU_PREFIX}-nm"

echo "=== Build complete ==="
ls -la "$OUT/spu_float_edges.elf" "$OUT/spu_main.elf"
