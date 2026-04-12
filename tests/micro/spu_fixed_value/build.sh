#!/bin/bash
# Build the spu_fixed_value microtest.
#
# Run inside the ps3dev Docker container with the test source
# mounted at /src:
#
#   docker run --rm -v /path/to/spu_fixed_value:/src ps3dev-fresh bash /src/build.sh
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

COMMON=/src/../common

echo "=== Assembling custom CRT0 ==="
${PPU_PREFIX}-gcc \
    -c -o "$OUT/crt0.o" \
    "$COMMON/crt0.S"

echo "=== Linking PPU program ==="
# Use -nostartfiles to skip PSL1GHT's CRT0 (which uses rldicr
# instructions that RPCS3 mishandles) and link our custom CRT0.
# The SPU ELF is loaded at runtime via sysSpuImageOpenELF rather
# than embedded, which avoids RPCS3's block analyzer misreading
# SPU data as PPU instructions.
${PPU_PREFIX}-gcc \
    -nostartfiles \
    -I${PSL1GHT}/ppu/include \
    -L${PSL1GHT}/ppu/lib \
    -O2 -Wall \
    -o "$OUT/spu_fixed_value.elf" \
    "$OUT/crt0.o" \
    /src/ppu/main.c \
    -llv2 -lsysmodule -lrt

echo "=== Patching TOC and rldicr ==="
python3 "$COMMON/patch_toc.py" \
    "$OUT/spu_fixed_value.elf" \
    "${PPU_PREFIX}-readelf" \
    "${PPU_PREFIX}-nm"

echo "=== Build complete ==="
ls -la "$OUT/spu_fixed_value.elf" "$OUT/spu_main.elf"
