/* PPU probe: what the PRX start, stop and unload calls answer for each
 * module state, and how they read the command word. The module is the
 * firmware's libadec.sprx, which loads and starts in a game process
 * (the lv2_prx_load_cycle capture).
 *
 * Every raw start/stop call sends a 0x28-byte option: size 0x28, cmd,
 * entry 0x1111111111111111, res, entry2 0x2222222222222222, so a write
 * to either entry slot shows.
 *
 * Block 1, one module A (slots 0-16):
 *   0  sysPrxLoadModule(libadec)                -> id A
 *   1  raw 482(A, cmd 1)       before any start
 *   2  raw 482(A, cmd 4)       before any start
 *   3  raw 482(A, cmd 2, res 0) before any start
 *   4  sysPrxStartModule(A)                     rc
 *   5  modres of step 4
 *   6  sysPrxStartModule(A) again               rc
 *   7  raw 481(A, cmd 1)       once started
 *   8  raw 483(A)              once started
 *   9  sysPrxStopModule(A)                      rc
 *  10  modres of step 9
 *  11  sysPrxStopModule(A) again                rc
 *  12  raw 482(A, cmd 1)       once stopped
 *  13  raw 482(A, cmd 4)       once stopped
 *  14  raw 483(A)                               rc
 *  15  raw 483(A) again                         rc
 *  16  raw 482(A, cmd 1)       once unloaded
 *
 * Block 2, slots 17-46: for cmd 1, 0x11, 0x21, 3, 0x10, 0, in order, a
 * fresh load B of libadec and five words:
 *   id B, raw 481(B, cmd) rc, entry, entry2, raw 483(B) rc
 *
 * Block 3, slots 47-70: for cmd 4, 0x14, 8, in order, a fresh load C,
 * sysPrxStartModule(C), then eight words:
 *   id C, start rc, raw 482(C, cmd) rc, entry, entry2,
 *   sysPrxStopModule(C) rc, raw 483(C) rc, raw 483(C) again rc
 *
 * A step whose id is an error records 0xFFFFFFFFFFFFFFFF in the slots
 * that need it.
 *
 * Payload layout: 71 big-endian u64, 568 bytes.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

/* liblv2's PRX entry points. PSL1GHT's sys/prx.h and lv2/prx.h share
 * one include guard, so neither header declares them here. */
typedef s32 sysPrxId;
sysPrxId sysPrxLoadModule(const char *path, u64 flags, void *opt);
s32 sysPrxStartModule(sysPrxId id, size_t args, void *argp, s32 *modres,
                      u64 flags, void *opt);
s32 sysPrxStopModule(sysPrxId id, size_t args, void *argp, s32 *modres,
                     u64 flags, void *opt);

#define SYS_PRX_START_MODULE 481
#define SYS_PRX_STOP_MODULE 482
#define SYS_PRX_UNLOAD_MODULE 483
#define STEPS 71
#define SKIPPED 0xFFFFFFFFFFFFFFFFULL
#define ENTRY_SEED 0x1111111111111111ULL
#define ENTRY2_SEED 0x2222222222222222ULL

static const char ADEC[] = "/dev_flash/sys/external/libadec.sprx";

static const u64 START_CMDS[] = { 1, 0x11, 0x21, 3, 0x10, 0 };
static const u64 STOP_CMDS[] = { 4, 0x14, 8 };

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile u64 result[STEPS] __attribute__((aligned(128)));
static volatile u64 opt[5] __attribute__((aligned(16)));

static void write_frame(const volatile void *data, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)data, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)data, len);
}

static u64 raw_phase(u64 number, sysPrxId id, u64 cmd, u64 res)
{
    opt[0] = 0x28;
    opt[1] = cmd;
    opt[2] = ENTRY_SEED;
    opt[3] = res;
    opt[4] = ENTRY2_SEED;
    return (u64)cgov_sc(number, (u64)(u32)id, 0, (u64)(unsigned long)opt,
                        0, 0, 0, 0, 0);
}

static u64 raw_unload(sysPrxId id)
{
    return (u64)cgov_sc(SYS_PRX_UNLOAD_MODULE, (u64)(u32)id, 0, 0, 0, 0, 0,
                        0, 0);
}

int main(void)
{
    sysPrxId a, b, c;
    s32 modres;
    unsigned int i, k, at;

    for (i = 0; i < STEPS; i++)
        result[i] = SKIPPED;

    a = sysPrxLoadModule(ADEC, 0, 0);
    result[0] = (u64)(s64)a;
    if (a >= 0) {
        result[1] = raw_phase(SYS_PRX_STOP_MODULE, a, 1, 0);
        result[2] = raw_phase(SYS_PRX_STOP_MODULE, a, 4, 0);
        result[3] = raw_phase(SYS_PRX_STOP_MODULE, a, 2, 0);
        modres = 0;
        result[4] = (u64)(s64)sysPrxStartModule(a, 0, 0, &modres, 0, 0);
        result[5] = (u64)(s64)modres;
        modres = 0;
        result[6] = (u64)(s64)sysPrxStartModule(a, 0, 0, &modres, 0, 0);
        result[7] = raw_phase(SYS_PRX_START_MODULE, a, 1, 0);
        result[8] = raw_unload(a);
        modres = 0;
        result[9] = (u64)(s64)sysPrxStopModule(a, 0, 0, &modres, 0, 0);
        result[10] = (u64)(s64)modres;
        modres = 0;
        result[11] = (u64)(s64)sysPrxStopModule(a, 0, 0, &modres, 0, 0);
        result[12] = raw_phase(SYS_PRX_STOP_MODULE, a, 1, 0);
        result[13] = raw_phase(SYS_PRX_STOP_MODULE, a, 4, 0);
        result[14] = raw_unload(a);
        result[15] = raw_unload(a);
        result[16] = raw_phase(SYS_PRX_STOP_MODULE, a, 1, 0);
    }

    at = 17;
    for (k = 0; k < sizeof(START_CMDS) / sizeof(START_CMDS[0]); k++, at += 5) {
        b = sysPrxLoadModule(ADEC, 0, 0);
        result[at] = (u64)(s64)b;
        if (b < 0)
            continue;
        result[at + 1] = raw_phase(SYS_PRX_START_MODULE, b, START_CMDS[k], 0);
        result[at + 2] = opt[2];
        result[at + 3] = opt[4];
        result[at + 4] = raw_unload(b);
    }

    at = 47;
    for (k = 0; k < sizeof(STOP_CMDS) / sizeof(STOP_CMDS[0]); k++, at += 8) {
        c = sysPrxLoadModule(ADEC, 0, 0);
        result[at] = (u64)(s64)c;
        if (c < 0)
            continue;
        modres = 0;
        result[at + 1] = (u64)(s64)sysPrxStartModule(c, 0, 0, &modres, 0, 0);
        result[at + 2] = raw_phase(SYS_PRX_STOP_MODULE, c, STOP_CMDS[k], 0);
        result[at + 3] = opt[2];
        result[at + 4] = opt[4];
        modres = 0;
        result[at + 5] = (u64)(s64)sysPrxStopModule(c, 0, 0, &modres, 0, 0);
        result[at + 6] = raw_unload(c);
        result[at + 7] = raw_unload(c);
    }

    write_frame(result, sizeof(result));
    return 0;
}
