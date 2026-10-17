/* PPU probe: the PRX start, stop and unload answers the lv2_prx_states
 * capture left open. A stop report sent to a never-started module moved
 * it out of reach there, so each question here gets its own load of the
 * firmware's libadec.sprx.
 *
 * Every raw start/stop call sends a 0x28-byte option: size 0x28, cmd,
 * entry 0x1111111111111111, res, entry2 0x2222222222222222.
 *
 * Module A, started through liblv2 (slots 0-14):
 *   0  sysPrxLoadModule                        -> id A
 *   1  sysPrxStartModule(A)                    rc
 *   2  sysPrxStartModule(A) again              rc
 *   3  raw 481(A, cmd 1)      once started     rc
 *   4  entry after step 3
 *   5  raw 481(A, cmd 2, res 0) once started   rc
 *   6  raw 483(A)             once started     rc
 *   7  sysPrxStopModule(A)                     rc
 *   8  sysPrxStopModule(A) again               rc
 *   9  raw 482(A, cmd 1)      once stopped     rc
 *  10  raw 482(A, cmd 4)      once stopped     rc
 *  11  raw 482(A, cmd 2, res 0) once stopped   rc
 *  12  raw 481(A, cmd 1)      once stopped     rc
 *  13  raw 483(A)                              rc
 *  14  raw 483(A) again                        rc
 *
 * Module D, a stop report before any start (slots 15-20):
 *  15  sysPrxLoadModule                        -> id D
 *  16  raw 482(D, cmd 2, res 0)                rc
 *  17  raw 482(D, cmd 1)                       rc
 *  18  raw 482(D, cmd 4)                       rc
 *  19  raw 481(D, cmd 2, res 0)                rc
 *  20  raw 483(D)                              rc
 *
 * Module E, a stop that cannot finish (slots 21-30):
 *  21  sysPrxLoadModule                        -> id E
 *  22  sysPrxStartModule(E)                    rc
 *  23  raw 482(E, cmd 1)                       rc
 *  24  entry after step 23
 *  25  raw 482(E, cmd 1) again                 rc
 *  26  raw 482(E, cmd 4)                       rc
 *  27  raw 482(E, cmd 2, res 1)                rc
 *  28  raw 482(E, cmd 1)                       rc
 *  29  raw 482(E, cmd 2, res 0)                rc
 *  30  raw 483(E)                              rc
 *
 * Module F, a start whose entry asks not to stay (slots 31-36):
 *  31  sysPrxLoadModule                        -> id F
 *  32  raw 481(F, cmd 1)                       rc
 *  33  raw 481(F, cmd 2, res 1)                rc
 *  34  raw 482(F, cmd 1)                       rc
 *  35  raw 483(F)                              rc
 *  36  raw 483(F) again                        rc
 *
 * A step whose id is an error records 0xFFFFFFFFFFFFFFFF in the slots
 * that need it.
 *
 * Payload layout: 37 big-endian u64, 296 bytes.
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

#define START 481
#define STOP 482
#define UNLOAD 483
#define STEPS 37
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

static u64 stop(sysPrxId id)
{
    s32 modres = 0;
    return (u64)(s64)sysPrxStopModule(id, 0, 0, &modres, 0, 0);
}

int main(void)
{
    sysPrxId a, d, e, f;
    unsigned int i;

    for (i = 0; i < STEPS; i++)
        result[i] = SKIPPED;

    a = sysPrxLoadModule(ADEC, 0, 0);
    result[0] = (u64)(s64)a;
    if (a >= 0) {
        result[1] = start(a);
        result[2] = start(a);
        result[3] = raw(START, a, 1, 0);
        result[4] = opt[2];
        result[5] = raw(START, a, 2, 0);
        result[6] = unload(a);
        result[7] = stop(a);
        result[8] = stop(a);
        result[9] = raw(STOP, a, 1, 0);
        result[10] = raw(STOP, a, 4, 0);
        result[11] = raw(STOP, a, 2, 0);
        result[12] = raw(START, a, 1, 0);
        result[13] = unload(a);
        result[14] = unload(a);
    }

    d = sysPrxLoadModule(ADEC, 0, 0);
    result[15] = (u64)(s64)d;
    if (d >= 0) {
        result[16] = raw(STOP, d, 2, 0);
        result[17] = raw(STOP, d, 1, 0);
        result[18] = raw(STOP, d, 4, 0);
        result[19] = raw(START, d, 2, 0);
        result[20] = unload(d);
    }

    e = sysPrxLoadModule(ADEC, 0, 0);
    result[21] = (u64)(s64)e;
    if (e >= 0) {
        result[22] = start(e);
        result[23] = raw(STOP, e, 1, 0);
        result[24] = opt[2];
        result[25] = raw(STOP, e, 1, 0);
        result[26] = raw(STOP, e, 4, 0);
        result[27] = raw(STOP, e, 2, 1);
        result[28] = raw(STOP, e, 1, 0);
        result[29] = raw(STOP, e, 2, 0);
        result[30] = unload(e);
    }

    f = sysPrxLoadModule(ADEC, 0, 0);
    result[31] = (u64)(s64)f;
    if (f >= 0) {
        result[32] = raw(START, f, 1, 0);
        result[33] = raw(START, f, 2, 1);
        result[34] = raw(STOP, f, 1, 0);
        result[35] = unload(f);
        result[36] = unload(f);
    }

    write_frame(result, sizeof(result));
    return 0;
}
