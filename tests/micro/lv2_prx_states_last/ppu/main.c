/* PPU probe: which call removed a started module in the
 * lv2_prx_states_more capture, and what a start report other than 0 and
 * 1 answers. Each question gets its own load of the firmware's
 * libadec.sprx.
 *
 * Every raw start/stop call sends a 0x28-byte option: size 0x28, cmd,
 * entry 0x1111111111111111, res, entry2 0x2222222222222222.
 *
 * Module A, an unload refused while started (slots 0-5):
 *   0  sysPrxLoadModule                        -> id A
 *   1  sysPrxStartModule(A)                    rc
 *   2  raw 483(A)                              rc
 *   3  raw 482(A, cmd 1)                       rc
 *   4  raw 482(A, cmd 2, res 0)                rc
 *   5  raw 483(A)                              rc
 *
 * Module B, a start report to a started module (slots 6-11):
 *   6  sysPrxLoadModule                        -> id B
 *   7  sysPrxStartModule(B)                    rc
 *   8  raw 481(B, cmd 2, res 0)                rc
 *   9  raw 482(B, cmd 1)                       rc
 *  10  raw 482(B, cmd 2, res 0)                rc
 *  11  raw 483(B)                              rc
 *
 * Module C, a start report of 5 (slots 12-16):
 *  12  sysPrxLoadModule                        -> id C
 *  13  raw 481(C, cmd 1)                       rc
 *  14  raw 481(C, cmd 2, res 5)                rc
 *  15  raw 482(C, cmd 1)                       rc
 *  16  raw 483(C)                              rc
 *
 * Module G, an unload refused between the two start phases
 * (slots 17-22):
 *  17  sysPrxLoadModule                        -> id G
 *  18  raw 481(G, cmd 1)                       rc
 *  19  raw 483(G)                              rc
 *  20  raw 482(G, cmd 1)                       rc
 *  21  raw 481(G, cmd 2, res 0)                rc
 *  22  raw 483(G)                              rc
 *
 * A step whose id is an error records 0xFFFFFFFFFFFFFFFF in the slots
 * that need it.
 *
 * Payload layout: 23 big-endian u64, 184 bytes.
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

#define START 481
#define STOP 482
#define UNLOAD 483
#define STEPS 23
#define SKIPPED 0xFFFFFFFFFFFFFFFFULL

static const char ADEC[] = "/dev_flash/sys/external/libadec.sprx";

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

static u64 raw(u64 number, sysPrxId id, u64 cmd, u64 res)
{
    opt[0] = 0x28;
    opt[1] = cmd;
    opt[2] = 0x1111111111111111ULL;
    opt[3] = res;
    opt[4] = 0x2222222222222222ULL;
    return (u64)cgov_sc(number, (u64)(u32)id, 0, (u64)(unsigned long)opt,
                        0, 0, 0, 0, 0);
}

static u64 unload(sysPrxId id)
{
    return (u64)cgov_sc(UNLOAD, (u64)(u32)id, 0, 0, 0, 0, 0, 0, 0);
}

static u64 start(sysPrxId id)
{
    s32 modres = 0;
    return (u64)(s64)sysPrxStartModule(id, 0, 0, &modres, 0, 0);
}

int main(void)
{
    sysPrxId a, b, c, g;
    unsigned int i;

    for (i = 0; i < STEPS; i++)
        result[i] = SKIPPED;

    a = sysPrxLoadModule(ADEC, 0, 0);
    result[0] = (u64)(s64)a;
    if (a >= 0) {
        result[1] = start(a);
        result[2] = unload(a);
        result[3] = raw(STOP, a, 1, 0);
        result[4] = raw(STOP, a, 2, 0);
        result[5] = unload(a);
    }

    b = sysPrxLoadModule(ADEC, 0, 0);
    result[6] = (u64)(s64)b;
    if (b >= 0) {
        result[7] = start(b);
        result[8] = raw(START, b, 2, 0);
        result[9] = raw(STOP, b, 1, 0);
        result[10] = raw(STOP, b, 2, 0);
        result[11] = unload(b);
    }

    c = sysPrxLoadModule(ADEC, 0, 0);
    result[12] = (u64)(s64)c;
    if (c >= 0) {
        result[13] = raw(START, c, 1, 0);
        result[14] = raw(START, c, 2, 5);
        result[15] = raw(STOP, c, 1, 0);
        result[16] = unload(c);
    }

    g = sysPrxLoadModule(ADEC, 0, 0);
    result[17] = (u64)(s64)g;
    if (g >= 0) {
        result[18] = raw(START, g, 1, 0);
        result[19] = unload(g);
        result[20] = raw(STOP, g, 1, 0);
        result[21] = raw(START, g, 2, 0);
        result[22] = unload(g);
    }

    write_frame(result, sizeof(result));
    return 0;
}
