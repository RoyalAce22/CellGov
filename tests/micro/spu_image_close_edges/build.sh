#!/bin/bash
# Build the spu_image_close_edges probe: two imports of one ELF, the
# close of one, and the close of a user image record.
#
# Requirements -- any environment providing:
#   1. the ps3dev toolchains (powerpc64-ps3-elf-gcc and spu-gcc) and
#      PSL1GHT, installed under $PS3DEV / $PSL1GHT (default
#      /usr/local/ps3dev, the ps3toolchain standard prefix;
#      override via env),
#   2. python3 (for common/patch_toc.py),
#   3. this test directory mounted at /src and the shared
#      tests/micro/common at /common (or as /src/../common). The PPU
#      program includes /src/build/spu_blob.h and /src/build/spu_blob.bin
#      by those absolute paths.
#
#   docker run --rm -v /path/to/spu_image_close_edges:/src \
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
COMMON="${COMMON:-/src/../common}"
mkdir -p "$OUT"

echo "=== Building SPU program ==="
${SPU_PREFIX}-gcc \
    -I${PSL1GHT}/spu/include \
    -L${PSL1GHT}/spu/lib \
    -O2 -Wall \
    -o "$OUT/spu_main.elf" \
    /src/spu/main.c \
    -lsputhread

echo "=== Flattening the SPU program into one local-store blob ==="
# The blob starts at the first PT_LOAD's address; objcopy fills any gap
# between segments with zeros, as a local store loaded segment by
# segment would hold.
${SPU_PREFIX}-objcopy -O binary "$OUT/spu_main.elf" "$OUT/spu_blob.bin"
LS=$(${SPU_PREFIX}-readelf -l -W "$OUT/spu_main.elf" | awk '$1 == "LOAD" { print $3; exit }')
ENTRY=$(${SPU_PREFIX}-readelf -h "$OUT/spu_main.elf" | awk '/Entry point address/ { print $4 }')
[ -n "$LS" ] && [ -n "$ENTRY" ] || { echo "no PT_LOAD or entry in spu_main.elf"; exit 1; }
cat > "$OUT/spu_blob.h" <<EOF
/* Written by build.sh from spu_main.elf. */
#define SPU_BLOB_LS    ${LS}u
#define SPU_BLOB_ENTRY ${ENTRY}u
EOF
cat "$OUT/spu_blob.h"

echo "=== Assembling custom CRT0 ==="
${PPU_PREFIX}-gcc -c -o "$OUT/crt0.o" "$COMMON/crt0.S"

echo "=== Linking PPU program ==="
${PPU_PREFIX}-gcc \
    -nostartfiles \
    -I${PSL1GHT}/ppu/include \
    -I"$COMMON" \
    -L${PSL1GHT}/ppu/lib \
    -O2 -Wall \
    -o "$OUT/spu_image_close_edges.elf" \
    "$OUT/crt0.o" \
    /src/ppu/main.c \
    -llv2 -lsysmodule -lrt

echo "=== Patching TOC ==="
python3 "$COMMON/patch_toc.py" \
    "$OUT/spu_image_close_edges.elf" \
    "${PPU_PREFIX}-readelf" \
    "${PPU_PREFIX}-nm"

echo "=== Build complete ==="
ls -la "$OUT/spu_image_close_edges.elf" "$OUT/spu_blob.bin"

bash "$COMMON/package_ps3.sh" spu_image_close_edges /src/ppu/main.c
