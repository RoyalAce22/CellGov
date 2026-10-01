#!/bin/bash
# Build the spu_events microtest: a hardware probe of the SPU events
# and the decrementer.
#
# Requirements -- any environment providing:
#   1. the ps3dev toolchains (powerpc64-ps3-elf-gcc and spu-gcc) and
#      PSL1GHT, installed under $PS3DEV / $PSL1GHT (default
#      /usr/local/ps3dev; override via env),
#   2. python3 (for common/patch_toc.py),
#   3. this test directory mounted at /src and the shared
#      tests/micro/common at /common (or as /src/../common).
#
#   docker run --rm -v /path/to/spu_events:/src \
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
    /src/spu/main.c \
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
    -I"$COMMON" \
    -L${PSL1GHT}/ppu/lib \
    -O2 -Wall \
    -o "$OUT/spu_events.elf" \
    "$OUT/crt0.o" \
    /src/ppu/main.c \
    -llv2 -lsysmodule -lrt

echo "=== Patching TOC and rldicr ==="
python3 "$COMMON/patch_toc.py" \
    "$OUT/spu_events.elf" \
    "${PPU_PREFIX}-readelf" \
    "${PPU_PREFIX}-nm"

echo "=== Build complete ==="
ls -la "$OUT/spu_events.elf" "$OUT/spu_main.elf"

bash "$COMMON/package_ps3.sh" spu_events /src/ppu/main.c "$OUT/spu_main.elf"
