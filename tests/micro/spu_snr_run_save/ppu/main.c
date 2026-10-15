/* PPU program: what the start of an SPU thread loads into a
 * signal-notification register no write reached, and whether a
 * sys_spu_thread_write_snr (184) made while the thread runs reaches the
 * value a later start loads.
 *
 * One one-thread group, configuration CFG (register 1 in OR mode), runs
 * spu/main.c twice with no write before either start. Each run reports
 * both registers (channel count, and the value read when the count is
 * nonzero) into SCRATCH, waits on its inbound mailbox, reports both
 * again into SCRATCH + 16 and exits. In the first run the PPU waits for
 * the first report, writes RUN_VALUE to register 1, then writes the
 * mailbox; in the second it writes only the mailbox.
 *
 * Result layout (96 bytes, big-endian u32 words):
 *   w0: failed step (0 = none), w1: its return code
 *   w2-w5:   run 1 first report: count 1, value 1, count 2, value 2
 *   w6:      write_snr(t, 0, RUN_VALUE) during run 1
 *   w7:      write_spu_mb during run 1
 *   w8-w11:  run 1 second report
 *   w12-w15: run 2 first report
 *   w16:     write_spu_mb during run 2
 *   w17-w20: run 2 second report
 *   w21:     destroy
 *   w22-w23: 0
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

#define RESULT_BYTES 96

#define SYS_SPU_THREAD_WRITE_SNR    184
#define SYS_SPU_THREAD_SET_SPU_CFG  187
#define SYS_SPU_THREAD_WRITE_SPU_MB 190

#define CFG        1
#define RUN_VALUE  0x2
#define UNWRITTEN  0xEEEEEEEEu
#define SPIN_LIMIT 100000000u

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));
static volatile unsigned int scratch[8] __attribute__((aligned(128)));

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

/* Wait until the SPU's first report lands; 0 when it never does. */
static int first_report_landed(void)
{
    unsigned int n;

    for (n = 0; n < SPIN_LIMIT; n++) {
        if (scratch[0] != UNWRITTEN)
            return 1;
    }
    return 0;
}

/* One run: start, wait for the first report, optionally write register
 * 1, release the SPU through its mailbox, join, and copy both reports. */
static int run(sys_spu_group_t group, sys_spu_thread_t thread, int write_during,
               unsigned int first_at, unsigned int second_at, unsigned int step)
{
    unsigned int cause, status, i;
    int ret;

    for (i = 0; i < 8; i++)
        scratch[i] = UNWRITTEN;
    __sync_synchronize();
    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(step, ret);
    if (!first_report_landed()) return fail(step + 1, 0);
    if (write_during) {
        result_buf[6] = (unsigned int)syscall3_s32(SYS_SPU_THREAD_WRITE_SNR, thread, 0, RUN_VALUE);
        result_buf[7] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_WRITE_SPU_MB, thread, 1);
    } else {
        result_buf[16] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_WRITE_SPU_MB, thread, 1);
    }
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(step + 2, ret);
    __sync_synchronize();
    for (i = 0; i < 4; i++) {
        result_buf[first_at + i] = scratch[i];
        result_buf[second_at + i] = scratch[4 + i];
    }
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
    g_grpattr.name = "snrsave";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &g_grpattr);
    if (ret != 0) return fail(2, ret);

    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 8;
    g_thrattr.name = "snrsave";
    memset(&g_args, 0, sizeof(g_args));
    g_args.arg0 = (u64)(unsigned long)scratch;
    ret = sysSpuThreadInitialize(&thread, group, 0, &g_image, &g_thrattr, &g_args);
    if (ret != 0) return fail(3, ret);
    ret = syscall2_s32(SYS_SPU_THREAD_SET_SPU_CFG, thread, CFG);
    if (ret != 0) return fail(4, ret);

    if (run(group, thread, 1, 2, 8, 5) != 0) return 1;
    if (run(group, thread, 0, 12, 17, 8) != 0) return 1;

    result_buf[21] = (unsigned int)sysSpuThreadGroupDestroy(group);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
