/* PPU program: where a sys_spu_thread_write_snr (184) to a thread with
 * no running SPU goes.
 *
 * One one-thread group runs spu/main.c, which reports the channel count
 * of each signal-notification register and, when the count is nonzero,
 * the value it reads, into SCRATCH; the PPU copies SCRATCH out after
 * each join. The thread's configuration is CFG: register 1 in OR mode,
 * register 2 in overwrite mode.
 *
 * Result layout (64 bytes, big-endian u32 words):
 *   w0: failed step (0 = none), w1: its return code
 *   Before the first start:
 *   w2: write_snr(t, 0, 0x10)   w3: write_snr(t, 0, 0x01)
 *   w4: write_snr(t, 1, 0x20)   w5: write_snr(t, 1, 0x40)
 *   w6-w9: first run: register 1 count, its value, register 2 count,
 *          its value (0xFFFFFFFF for a value not read)
 *   After the first run ended, before the second start:
 *   w10: write_snr(t, 0, 0x100)
 *   w11-w14: second run, as w6-w9
 *   w15: destroy
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 64

#define SYS_SPU_THREAD_WRITE_SNR   184
#define SYS_SPU_THREAD_SET_SPU_CFG 187

#define CFG 1

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));
static volatile unsigned int scratch[4] __attribute__((aligned(128)));

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

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

static sysSpuImage g_image;
static sysSpuThreadGroupAttribute g_grpattr;
static sysSpuThreadAttribute g_thrattr;
static sysSpuThreadArgument g_args;

static unsigned int write_snr(u64 thread, u64 number, u64 value)
{
    return (unsigned int)syscall3_s32(SYS_SPU_THREAD_WRITE_SNR, thread, number, value);
}

/* Start the group, join it, and copy SCRATCH to result words `at`.. */
static int run(sys_spu_group_t group, unsigned int at, unsigned int step)
{
    unsigned int cause, status, i;
    int ret;

    for (i = 0; i < 4; i++)
        scratch[i] = 0xEEEEEEEE;
    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(step, ret);
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(step + 1, ret);
    __sync_synchronize();
    for (i = 0; i < 4; i++)
        result_buf[at + i] = scratch[i];
    return 0;
}

int main(void)
{
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    int ret;

    memset((void *)result_buf, 0, sizeof(result_buf));

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1, ret);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "snrout";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &g_grpattr);
    if (ret != 0) return fail(2, ret);

    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 8;
    g_thrattr.name = "snrout";
    memset(&g_args, 0, sizeof(g_args));
    g_args.arg0 = (u64)(unsigned long)scratch;
    ret = sysSpuThreadInitialize(&thread, group, 0, &g_image, &g_thrattr, &g_args);
    if (ret != 0) return fail(3, ret);
    ret = syscall2_s32(SYS_SPU_THREAD_SET_SPU_CFG, thread, CFG);
    if (ret != 0) return fail(4, ret);

    result_buf[2] = write_snr(thread, 0, 0x10);
    result_buf[3] = write_snr(thread, 0, 0x01);
    result_buf[4] = write_snr(thread, 1, 0x20);
    result_buf[5] = write_snr(thread, 1, 0x40);
    if (run(group, 6, 5) != 0) return 1;

    result_buf[10] = write_snr(thread, 0, 0x100);
    if (run(group, 11, 7) != 0) return 1;

    result_buf[15] = (unsigned int)sysSpuThreadGroupDestroy(group);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
