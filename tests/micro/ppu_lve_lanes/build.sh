#!/bin/bash
# Build the ppu_lve_lanes probe: what the vector element loads leave in
# the lanes of vD they do not name.
#
# Requirements -- any environment providing:
#   1. the ps3dev PPU toolchain (powerpc64-ps3-elf-gcc) and PSL1GHT,
#      installed under $PS3DEV / $PSL1GHT (default /usr/local/ps3dev;
#      override via env),
#   2. python3 (for common/patch_toc.py),
#   3. this test directory mounted at /src and the shared
#      tests/micro/common at /common (or as /src/../common).
#
#   docker run --rm -v /path/to/ppu_lve_lanes:/src \
#       -v /path/to/common:/common \
#       -e COMMON=/common ps3dev bash /src/build.sh
#
# Git Bash on Windows rewrites the /src and /common mount targets;
# prefix the command with MSYS_NO_PATHCONV=1 there.
#
set -e

PS3DEV="${PS3DEV:-/usr/local/ps3dev}"
PSL1GHT="${PSL1GHT:-$PS3DEV}"
PPU_PREFIX="powerpc64-ps3-elf"

OUT=/src/build
COMMON="${COMMON:-/src/../common}"
mkdir -p "$OUT"

${PPU_PREFIX}-gcc -c -o "$OUT/crt0.o" "$COMMON/crt0.S"
${PPU_PREFIX}-gcc -nostartfiles \
    -I${PSL1GHT}/ppu/include -I"$COMMON" -L${PSL1GHT}/ppu/lib \
    -O2 -Wall -maltivec \
    -o "$OUT/ppu_lve_lanes.elf" \
    "$OUT/crt0.o" /src/ppu/main.c \
    -llv2 -lsysmodule -lrt
python3 "$COMMON/patch_toc.py" \
    "$OUT/ppu_lve_lanes.elf" \
    "${PPU_PREFIX}-readelf" "${PPU_PREFIX}-nm"
ls -la "$OUT/ppu_lve_lanes.elf"

bash "$COMMON/package_ps3.sh" ppu_lve_lanes /src/ppu/main.c
