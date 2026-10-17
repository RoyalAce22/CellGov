/* PPU probe: what the PRX load, start, stop and unload calls answer for a
 * firmware path that does not exist, for the stop handshake's report
 * phase sent out of order to a started module, and for three modules
 * that export the same library.
 *
 * Steps, in payload order (one u64 each; an id is the u64 the call
 * returned, a negative one is its error code):
 *   0  raw sc 480 on /dev_flash/sys/external/cgov_missing.sprx
 *   1  sysPrxLoadModule(libadec.sprx)                     -> id A
 *   2  sysPrxStartModule(A)                               rc
 *   3  modres of step 2
 *   4  raw sc 482(A, size 0x28, cmd 2, res 5)            rc, no cmd 1 first
 *   5  raw sc 482(A, size 0x28, cmd 2, res -1)           rc
 *   6  raw sc 482(A, size 0x28, cmd 0x11)                rc
 *   7  entry word after step 6
 *   8  entry2 word after step 6
 *   9  sysPrxStopModule(A)                                rc
 *  10  modres of step 9
 *  11  sysPrxUnloadModule(A)                              rc
 *  12  sysPrxLoadModule(libadec.sprx)                     -> id B
 *  13  sysPrxStartModule(B)                               rc
 *  14  sysPrxLoadModule(libadec2.sprx)                    -> id C
 *  15  sysPrxStartModule(C)                               rc
 *  16  sysPrxLoadModule(libadec_internal.sprx)            -> id D
 *  17  sysPrxStartModule(D)                               rc
 *  18  sysPrxStopModule(D)    19  sysPrxUnloadModule(D)
 *  20  sysPrxStopModule(C)    21  sysPrxUnloadModule(C)
 *  22  sysPrxStopModule(B)    23  sysPrxUnloadModule(B)
 *
 * A step whose id is an error skips the calls that need it and records
 * 0xFFFFFFFFFFFFFFFF in their slots.
 *
 * Payload layout: 24 big-endian u64, 192 bytes.
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
s32 sysPrxUnloadModule(sysPrxId id, u64 flags, void *opt);

#define SYS_PRX_LOAD_MODULE 480
#define SYS_PRX_STOP_MODULE 482
#define STEPS 24
#define SKIPPED 0xFFFFFFFFFFFFFFFFULL

static const char MISSING[] = "/dev_flash/sys/external/cgov_missing.sprx";
static const char ADEC[] = "/dev_flash/sys/external/libadec.sprx";
static const char ADEC2[] = "/dev_flash/sys/external/libadec2.sprx";
static const char ADEC_INTERNAL[] = "/dev_flash/sys/external/libadec_internal.sprx";

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

static u64 raw_stop(sysPrxId id, u64 cmd, u64 res)
{
    opt[0] = 0x28;
    opt[1] = cmd;
    opt[2] = 0x1111111111111111ULL;
    opt[3] = res;
    opt[4] = 0x2222222222222222ULL;
    return (u64)cgov_sc(SYS_PRX_STOP_MODULE, (u64)(u32)id, 0,
                        (u64)(unsigned long)opt, 0, 0, 0, 0, 0);
}

static void start(sysPrxId id, unsigned int slot)
{
    s32 modres = 0;
    if (id < 0) {
        result[slot] = SKIPPED;
        return;
    }
    result[slot] = (u64)(s64)sysPrxStartModule(id, 0, 0, &modres, 0, 0);
}

static void stop_unload(sysPrxId id, unsigned int slot)
{
    s32 modres = 0;
    if (id < 0) {
        result[slot] = SKIPPED;
        result[slot + 1] = SKIPPED;
        return;
    }
    result[slot] = (u64)(s64)sysPrxStopModule(id, 0, 0, &modres, 0, 0);
    result[slot + 1] = (u64)(s64)sysPrxUnloadModule(id, 0, 0);
}

int main(void)
{
    sysPrxId a, b, c, d;
    s32 modres;
    unsigned int i;

    for (i = 0; i < STEPS; i++)
        result[i] = SKIPPED;

    result[0] = (u64)cgov_sc(SYS_PRX_LOAD_MODULE, (u64)(unsigned long)MISSING,
                             0, 0, 0, 0, 0, 0, 0);

    a = sysPrxLoadModule(ADEC, 0, 0);
    result[1] = (u64)(s64)a;
    if (a >= 0) {
        modres = 0;
        result[2] = (u64)(s64)sysPrxStartModule(a, 0, 0, &modres, 0, 0);
        result[3] = (u64)(s64)modres;
        result[4] = raw_stop(a, 2, 5);
        result[5] = raw_stop(a, 2, (u64)-1LL);
        result[6] = raw_stop(a, 0x11, 0);
        result[7] = opt[2];
        result[8] = opt[4];
        modres = 0;
        result[9] = (u64)(s64)sysPrxStopModule(a, 0, 0, &modres, 0, 0);
        result[10] = (u64)(s64)modres;
        result[11] = (u64)(s64)sysPrxUnloadModule(a, 0, 0);
    }

    b = sysPrxLoadModule(ADEC, 0, 0);
    result[12] = (u64)(s64)b;
    start(b, 13);
    c = sysPrxLoadModule(ADEC2, 0, 0);
    result[14] = (u64)(s64)c;
    start(c, 15);
    d = sysPrxLoadModule(ADEC_INTERNAL, 0, 0);
    result[16] = (u64)(s64)d;
    start(d, 17);
    stop_unload(d, 18);
    stop_unload(c, 20);
    stop_unload(b, 22);

    write_frame(result, sizeof(result));
    return 0;
}
