#!/bin/bash
# Build the ppu_atomic_spinlock microtest.
#
# Run inside the ps3dev Docker container with the test source
# mounted at /src:
#
#   docker run --rm -v /path/to/ppu_atomic_spinlock:/src \
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
# -nostartfiles: skip PSL1GHT's CRT0 (uses rldicr, mishandled by
# RPCS3) and link our custom CRT0 from common/.
${PPU_PREFIX}-gcc \
    -nostartfiles \
    -I${PSL1GHT}/ppu/include \
    -L${PSL1GHT}/ppu/lib \
    -O2 -Wall \
    -o "$OUT/ppu_atomic_spinlock.elf" \
    "$OUT/crt0.o" \
    /src/ppu/main.c \
    -llv2 -lsysmodule -lrt

echo "=== Patching TOC and rldicr ==="
python3 "$COMMON/patch_toc.py" \
    "$OUT/ppu_atomic_spinlock.elf" \
    "${PPU_PREFIX}-readelf" \
    "${PPU_PREFIX}-nm"

echo "=== Build complete ==="
ls -la "$OUT/ppu_atomic_spinlock.elf"
