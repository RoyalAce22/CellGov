/* PPU program: a SPURS instance initialized through the firmware's
 * cellSpurs library, which starts the SPURS handler PPU threads and the
 * SPU thread group the SPURS kernel runs on, then finalized.
 *
 * No workload is ever added, so the kernel never runs a job.
 *
 * Result layout (128 bytes, big-endian u32 words; each word is the
 * call's return code unless named otherwise):
 *   w0:  failed step (0 = none), w1: its return code
 *   w2:  sys_spu_initialize(6, 0)
 *   w3:  load the SPURS system module
 *   w4:  initialize(1 SPU, SPU priority 250, PPU priority 1000, stay)
 *   w5:  get the SPU thread count, w6: the count
 *   w7:  get the SPU thread group id, w8: the id
 *   w9:  get the instance info
 *   w10: info SPU count, w11: info SPU group priority
 *   w12: info handler PPU priority, w13: info exit-if-no-work
 *   w14: 1 when info names handler PPU thread 0, else 0
 *   w15: 1 when info names handler PPU thread 1, else 0
 *   w16: 1 when info's SPU group id is w8, else 0
 *   w17: finalize
 *   w18: unload the SPURS system module
 *   w19-w31: 0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>
#include <sysmodule/sysmodule.h>
#include <spurs/spurs.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 128

#define SYS_SPU_INITIALIZE 169

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

static Spurs g_spurs;
static SpursInfo g_info;

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

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result_buf[0] = step;
    result_buf[1] = (unsigned int)rc;
    write_frame(result_buf, RESULT_BYTES);
    return (int)step;
}

int main(void)
{
    unsigned int n = 0xEEEEEEEEu, i;
    sys_spu_group_t group = 0xEEEEEEEEu;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;
    for (i = 19; i < RESULT_BYTES / 4; i++)
        result_buf[i] = 0;

    result_buf[2] = (unsigned int)syscall2_s32(SYS_SPU_INITIALIZE, 6, 0);
    /* A refused load still goes on: the SPURS library may already be
     * resident, and the initialize answers for itself. */
    result_buf[3] = (unsigned int)sysModuleLoad(SYSMODULE_SPURS);

    result_buf[4] = (unsigned int)spursInitialize(&g_spurs, 1, 250, 1000, false);
    if (result_buf[4] != 0) {
        sysModuleUnload(SYSMODULE_SPURS);
        return fail(2, (int)result_buf[4]);
    }

    result_buf[5] = (unsigned int)spursGetNumSpuThread(&g_spurs, &n);
    result_buf[6] = n;
    result_buf[7] = (unsigned int)spursGetSpuThreadGroupId(&g_spurs, &group);
    result_buf[8] = (unsigned int)group;

    memset(&g_info, 0, sizeof(g_info));
    result_buf[9] = (unsigned int)spursGetInfo(&g_spurs, &g_info);
    result_buf[10] = (unsigned int)g_info.nSpus;
    result_buf[11] = (unsigned int)g_info.spuGroupPriority;
    result_buf[12] = (unsigned int)g_info.ppuThreadPriority;
    result_buf[13] = g_info.exitIfNoWork ? 1u : 0u;
    result_buf[14] = g_info.spursHandlerThread0 != 0 ? 1u : 0u;
    result_buf[15] = g_info.spursHandlerThread1 != 0 ? 1u : 0u;
    result_buf[16] = (unsigned int)g_info.spuGroup == (unsigned int)group ? 1u : 0u;

    result_buf[17] = (unsigned int)spursFinalize(&g_spurs);
    result_buf[18] = (unsigned int)sysModuleUnload(SYSMODULE_SPURS);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
