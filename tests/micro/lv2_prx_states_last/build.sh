#!/bin/bash
# Build the lv2_prx_states_last probe: which call removed a started PRX,
# and what a start report other than 0 and 1 answers.
#
# Requirements -- any environment providing:
#   1. the ps3dev PPU toolchain (powerpc64-ps3-elf-gcc) and PSL1GHT,
#      installed under $PS3DEV / $PSL1GHT (default /usr/local/ps3dev;
#      override via env),
#   2. python3 (for common/patch_toc.py),
#   3. this test directory mounted at /src and the shared
#      tests/micro/common at /common (or as /src/../common).
#
#   docker run --rm -v /path/to/lv2_prx_states_last:/src \
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
    -O2 -Wall \
    -o "$OUT/lv2_prx_states_last.elf" \
    "$OUT/crt0.o" /src/ppu/main.c \
    -llv2 -lsysmodule -lrt
python3 "$COMMON/patch_toc.py" \
    "$OUT/lv2_prx_states_last.elf" \
    "${PPU_PREFIX}-readelf" "${PPU_PREFIX}-nm"
ls -la "$OUT/lv2_prx_states_last.elf"

bash "$COMMON/package_ps3.sh" lv2_prx_states_last /src/ppu/main.c
