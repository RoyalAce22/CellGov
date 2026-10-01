#!/bin/bash
# Package the PS3 variant of a microtest for a retail console.
#
#   package_ps3.sh <name> <ppu source> [sibling...]
#
# Called as the last line of a microtest's build.sh, inside the same
# ps3dev image. It leaves under /src/build/ps3/:
#
#   <name>.elf   the PPU program relinked with -DCGOV_OUT_FILE, so the
#                CGOV frame also lands in /dev_hdd0/tmp/cgov_<name>.bin
#                (tests/micro/common/cgov_out.h), TOC-patched like the
#                reference ELF, plus PSL1GHT's lv2-sprx.o so the ELF
#                carries the PRX parameter record the console's loader
#                and sprxlinker both need;
#   EBOOT.BIN    that ELF as an NPDRM SELF under one fixed content id;
#   PARAM.SFO    the title record the console needs beside it, with the
#                TITLE_ID this microtest deploys under;
#   link.txt     which start file the ELF was linked with;
#   siblings     each extra argument copied in, so a program the PPU
#                opens under /app_home/ (spu_main.elf, child.self) sits
#                beside the EBOOT.
#
# The reference build/<name>.elf is not touched: the emulator baselines
# were recorded from it and stay valid.
#
# Environment:
#   CGOV_PS3_APPID           TITLE_ID for PARAM.SFO (default CGOV00001)
#   CGOV_PS3_PSL1GHT_CRT=1   link with PSL1GHT's own start files instead
#                            of tests/micro/common/crt0.S, in case the
#                            custom start file does not boot on the console
set -e

PS3DEV="${PS3DEV:-/usr/local/ps3dev}"
PSL1GHT="${PSL1GHT:-$PS3DEV}"
PPU_PREFIX="powerpc64-ps3-elf"

NAME="${1:?usage: package_ps3.sh <name> <ppu source> [sibling...]}"
PPU_SOURCE="${2:?usage: package_ps3.sh <name> <ppu source> [sibling...]}"
shift 2

OUT=/src/build
PS3OUT="$OUT/ps3"
COMMON="${COMMON:-/src/../common}"
APPID="${CGOV_PS3_APPID:-CGOV00001}"
RESULT_FILE="/dev_hdd0/tmp/cgov_${NAME}.bin"
# One content id for every microtest: the console keys NPDRM on the
# EBOOT's own name, so the id need not vary per test.
CONTENT_ID="UP0001-CGOV00001_00-CELLGOVMICROTEST"

mkdir -p "$PS3OUT"

echo "=== Relinking PPU program for the console (result file $RESULT_FILE) ==="
if [ "${CGOV_PS3_PSL1GHT_CRT:-0}" = "1" ]; then
    START_FILES=()
    START_FLAGS=()
    LINK_KIND="psl1ght-crt"
else
    # lv2-sprx.o is one of PSL1GHT's default start files that
    # -nostartfiles drops. It holds no code, only the
    # .sys_proc_prx_param record that points at the import stub table;
    # sprxlinker refuses an ELF without it, and the console's loader
    # resolves the liblv2 / libsysmodule stubs through it.
    START_FILES=("$OUT/crt0.o" "$(${PPU_PREFIX}-gcc -print-file-name=lv2-sprx.o)")
    START_FLAGS=(-nostartfiles)
    LINK_KIND="common-crt0"
fi
${PPU_PREFIX}-gcc \
    "${START_FLAGS[@]}" \
    -I${PSL1GHT}/ppu/include \
    -I"$COMMON" \
    -L${PSL1GHT}/ppu/lib \
    -O2 -Wall \
    -DCGOV_OUT_FILE="\"$RESULT_FILE\"" \
    -o "$PS3OUT/$NAME.elf" \
    "${START_FILES[@]}" \
    "$PPU_SOURCE" \
    -llv2 -lsysmodule -lrt
if [ "$LINK_KIND" = "common-crt0" ]; then
    python3 "$COMMON/patch_toc.py" \
        "$PS3OUT/$NAME.elf" \
        "${PPU_PREFIX}-readelf" \
        "${PPU_PREFIX}-nm"
fi
echo "$LINK_KIND" > "$PS3OUT/link.txt"

echo "=== Wrapping EBOOT.BIN ==="
sprxlinker "$PS3OUT/$NAME.elf"
make_self_npdrm "$PS3OUT/$NAME.elf" "$PS3OUT/EBOOT.BIN" "$CONTENT_ID"

echo "=== Writing PARAM.SFO (TITLE_ID $APPID) ==="
sfo --fromxml "$COMMON/param_sfo.xml" "$PS3OUT/PARAM.SFO" \
    --title="CellGov $NAME" --appid="$APPID"

for sibling in "$@"; do
    echo "=== Copying sibling $(basename "$sibling") ==="
    cp "$sibling" "$PS3OUT/"
done

echo "=== PS3 package complete ==="
ls -la "$PS3OUT"
