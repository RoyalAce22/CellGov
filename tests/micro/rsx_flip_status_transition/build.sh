#!/bin/bash
# Build the rsx_flip_status_transition microtest.
#
# Run inside the ps3dev Docker container with the test source
# mounted at /src:
#
#   docker run --rm -v /path/to/rsx_flip_status_transition:/src \
#       -v /path/to/common:/common \
#       -e COMMON=/common ps3dev-fresh bash /src/build.sh
#
set -e

PS3DEV="${PS3DEV:-/usr/local/ps3dev}"
PSL1GHT="${PSL1GHT:-$PS3DEV}"
PPU_PREFIX="powerpc64-ps3-elf"

OUT=/src/build
COMMON="${COMMON:-/src/../common}"
mkdir -p "$OUT"

echo "=== Assembling custom CRT0 ==="
${PPU_PREFIX}-gcc -c -o "$OUT/crt0.o" "$COMMON/crt0.S"

echo "=== Linking PPU program ==="
${PPU_PREFIX}-gcc \
    -nostartfiles \
    -I${PSL1GHT}/ppu/include \
    -L${PSL1GHT}/ppu/lib \
    -O2 -Wall \
    -o "$OUT/rsx_flip_status_transition.elf" \
    "$OUT/crt0.o" \
    /src/ppu/main.c \
    -llv2 -lsysmodule -lrt

echo "=== Patching TOC and rldicr ==="
python3 "$COMMON/patch_toc.py" \
    "$OUT/rsx_flip_status_transition.elf" \
    "${PPU_PREFIX}-readelf" \
    "${PPU_PREFIX}-nm"

echo "=== Build complete ==="
ls -la "$OUT/rsx_flip_status_transition.elf"
