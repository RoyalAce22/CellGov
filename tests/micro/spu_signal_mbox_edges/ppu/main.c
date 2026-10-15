/* PPU program: the check order of sys_spu_thread_write_snr (184),
 * sys_spu_thread_set_spu_cfg (187) and sys_spu_thread_get_spu_cfg (188)
 * when both the thread id and a second argument are bad; what 184, 187,
 * 188 and sys_spu_thread_write_spu_mb (190) answer for a thread before
 * its group starts, while it runs and after it ends; and the mailbox
 * write that wakes a thread waiting on its inbound mailbox.
 *
 * One one-thread group runs spu/main.c, which reads its inbound mailbox
 * (waiting until the PPU writes it) and exits the thread with the value
 * it read. NO_SUCH_ID names no thread.
 *
 * Result layout (128 bytes, big-endian u32 words):
 *   w0: failed step (0 = none), w1: its return code
 *   Thread initialized, group not started:
 *   w2:  set_spu_cfg(NO_SUCH_ID, 4)
 *   w3:  set_spu_cfg(NO_SUCH_ID, 0)
 *   w4:  set_spu_cfg(t, 4)
 *   w5:  set_spu_cfg(t, 3)
 *   w6:  get_spu_cfg(t, &cfg), w7: cfg high word, w8: cfg low word
 *   w9:  get_spu_cfg(NO_SUCH_ID, NULL)
 *   w10: get_spu_cfg(t, NULL)
 *   w11: write_snr(NO_SUCH_ID, 2, SNR_VALUE)
 *   w12: write_snr(t, 0, SNR_VALUE)
 *   w13: write_snr(t, 2, SNR_VALUE)
 *   w14: write_spu_mb(NO_SUCH_ID, MBOX_VALUE)
 *   Group running, its thread waiting on its inbound mailbox:
 *   w15: get_spu_cfg(t, &cfg), w16: cfg low word
 *   w17: set_spu_cfg(t, 1)
 *   w18: get_spu_cfg(t, &cfg), w19: cfg low word
 *   w20: write_spu_mb(t, MBOX_VALUE)
 *   w21: join cause, w22: join status
 *   w23: get_exit_status(t, &status), w24: that status
 *   Group ended:
 *   w25: write_snr(t, 0, SNR_VALUE)
 *   w26: get_spu_cfg(t, &cfg), w27: cfg low word
 *   w28: destroy
 *   w29-w31: 0
 *
 * write_spu_mb is not sent to the thread before the start or after the
 * end: a value it took then would wait in a mailbox no thread reads.
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

#define RESULT_BYTES 128

#define SYS_SPU_THREAD_GET_EXIT_STATUS 165
#define SYS_SPU_THREAD_WRITE_SNR       184
#define SYS_SPU_THREAD_SET_SPU_CFG     187
#define SYS_SPU_THREAD_GET_SPU_CFG     188
#define SYS_SPU_THREAD_WRITE_SPU_MB    190

#define NO_SUCH_ID 0xFFFFFFFFu
#define SNR_VALUE  0x5A5A5A5Au
#define MBOX_VALUE 0x00C0FFEEu

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

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
static unsigned long long g_cfg;
static unsigned int g_status;

static unsigned int set_cfg(u64 thread, u64 value)
{
    return (unsigned int)syscall2_s32(SYS_SPU_THREAD_SET_SPU_CFG, thread, value);
}

static unsigned int get_cfg(u64 thread, u64 value_ptr)
{
    return (unsigned int)syscall2_s32(SYS_SPU_THREAD_GET_SPU_CFG, thread, value_ptr);
}

static unsigned int write_snr(u64 thread, u64 number, u64 value)
{
    return (unsigned int)syscall3_s32(SYS_SPU_THREAD_WRITE_SNR, thread, number, value);
}

static unsigned int write_mb(u64 thread, u64 value)
{
    return (unsigned int)syscall2_s32(SYS_SPU_THREAD_WRITE_SPU_MB, thread, value);
}

int main(void)
{
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    unsigned int cause, status;
    u64 cfg_ptr = (u64)(unsigned long)&g_cfg;
    int ret;

    memset((void *)result_buf, 0, sizeof(result_buf));

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1, ret);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "sigmbox";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &g_grpattr);
    if (ret != 0) return fail(2, ret);

    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 8;
    g_thrattr.name = "sigmbox";
    memset(&g_args, 0, sizeof(g_args));
    ret = sysSpuThreadInitialize(&thread, group, 0, &g_image, &g_thrattr, &g_args);
    if (ret != 0) return fail(3, ret);

    /* Thread initialized, group not started. */
    result_buf[2] = set_cfg(NO_SUCH_ID, 4);
    result_buf[3] = set_cfg(NO_SUCH_ID, 0);
    result_buf[4] = set_cfg(thread, 4);
    result_buf[5] = set_cfg(thread, 3);
    g_cfg = 0xFFFFFFFFFFFFFFFFull;
    result_buf[6] = get_cfg(thread, cfg_ptr);
    result_buf[7] = (unsigned int)(g_cfg >> 32);
    result_buf[8] = (unsigned int)g_cfg;
    result_buf[9] = get_cfg(NO_SUCH_ID, 0);
    result_buf[10] = get_cfg(thread, 0);
    result_buf[11] = write_snr(NO_SUCH_ID, 2, SNR_VALUE);
    result_buf[12] = write_snr(thread, 0, SNR_VALUE);
    result_buf[13] = write_snr(thread, 2, SNR_VALUE);
    result_buf[14] = write_mb(NO_SUCH_ID, MBOX_VALUE);

    /* Group running; its thread waits on its inbound mailbox. */
    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(4, ret);
    g_cfg = 0xFFFFFFFFFFFFFFFFull;
    result_buf[15] = get_cfg(thread, cfg_ptr);
    result_buf[16] = (unsigned int)g_cfg;
    result_buf[17] = set_cfg(thread, 1);
    g_cfg = 0xFFFFFFFFFFFFFFFFull;
    result_buf[18] = get_cfg(thread, cfg_ptr);
    result_buf[19] = (unsigned int)g_cfg;
    result_buf[20] = write_mb(thread, MBOX_VALUE);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(5, ret);
    result_buf[21] = cause;
    result_buf[22] = status;
    g_status = 0xFFFFFFFF;
    result_buf[23] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_GET_EXIT_STATUS, thread,
                                                (u64)(unsigned long)&g_status);
    result_buf[24] = g_status;

    /* Group ended. */
    result_buf[25] = write_snr(thread, 0, SNR_VALUE);
    g_cfg = 0xFFFFFFFFFFFFFFFFull;
    result_buf[26] = get_cfg(thread, cfg_ptr);
    result_buf[27] = (unsigned int)g_cfg;
    result_buf[28] = (unsigned int)sysSpuThreadGroupDestroy(group);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
