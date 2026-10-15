/* PPU program: what sys_spu_thread_group_suspend (174),
 * sys_spu_thread_group_resume (175) and sys_spu_thread_group_yield (176)
 * answer in each group state, a terminate of a suspended group, and a
 * start of a group that has already run and been joined.
 *
 * Group A runs one thread of spu/main.c, which waits until the group is
 * terminated. X is an exclusive non-context group (type 0x18) and P a
 * cooperate-with-system group (type 0x20, two threads); neither is
 * started. NO_SUCH_ID names no group.
 *
 * Result layout (128 bytes, big-endian u32 words; each word is the
 * call's return code unless named otherwise):
 *   w0:  failed step (0 = none), w1: its return code
 *   A initialized, not started:
 *   w2:  suspend(NO_SUCH_ID), w3: resume(NO_SUCH_ID), w4: yield(NO_SUCH_ID)
 *   w5:  suspend(A), w6: resume(A), w7: yield(A)
 *   A running:
 *   w8:  yield(A), w9: resume(A)
 *   w10: suspend(A), w11: suspend(A) again
 *   w12: yield(A), w13: resume(A), w14: resume(A) again
 *   w15: terminate(A, 1), w16: A join cause, w17: A join status
 *   A joined:
 *   w18: suspend(A), w19: resume(A), w20: yield(A)
 *   w21: start(A) again
 *   w22: suspend(A), w23: terminate(A, 2) while suspended
 *   w24: A join cause, w25: A join status
 *   the other types:
 *   w26: suspend(X), w27: resume(X), w28: yield(X)
 *   w29: suspend(P), w30: resume(P), w31: yield(P)
 *   a message written while suspended (group B, one thread that reads
 *   its inbound mailbox and exits with the message):
 *   w32: suspend(B) after the start, w33: write_spu_mb(0x5A) while
 *   suspended, w34: resume(B), w35: B join cause, w36: B join status,
 *   w37: get_exit_status, w38: the exit status it wrote, w39: 0
 *   B again after the join (a status of 0xFFFFFFFF means none written):
 *   w40: start(B), w41: get_exit_status before the new run exits,
 *   w42: the status it wrote, w43: write_spu_mb(0x6B), w44: join cause,
 *   w45: join status, w46: get_exit_status, w47: the status it wrote
 *   B run to its end (message 0x7C), no join, then started again:
 *   w48: start(B); when it answers 0, w49: write_spu_mb(0x8D), else
 *   w49 is untouched; w50: join cause, w51: join status,
 *   w52: get_exit_status, w53: the status it wrote, w54: 0,
 *   w55: destroy(B)
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

#define RESULT_BYTES 224

#define SYS_SPU_THREAD_GET_EXIT_STATUS  165
#define SYS_SPU_THREAD_GROUP_CREATE     170
#define SYS_SPU_THREAD_GROUP_DESTROY    171
#define SYS_SPU_THREAD_GROUP_START      173
#define SYS_SPU_THREAD_GROUP_SUSPEND    174
#define SYS_SPU_THREAD_GROUP_RESUME     175
#define SYS_SPU_THREAD_GROUP_YIELD      176
#define SYS_SPU_THREAD_GROUP_TERMINATE  177
#define SYS_SPU_THREAD_WRITE_SPU_MB     190

#define SYS_TIMER_USLEEP                141

#define MESSAGE   0x5Au
#define MESSAGE_2 0x6Bu
#define MESSAGE_3 0x7Cu
#define MESSAGE_4 0x8Du

/* Long enough for a thread that has its message to exit. */
#define SETTLE_US 100000

#define NO_SUCH_ID 0xFFFFFFFFu

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

static sysSpuImage g_image;
static sysSpuThreadGroupAttribute g_grpattr;
static sysSpuThreadAttribute g_thrattr;
static sysSpuThreadArgument g_args;

/* sys_spu_thread_group_attribute: nsize, name, type, option. */
static unsigned int g_attr[4] __attribute__((aligned(16)));
static const char GROUP_NAME[] = "susp";
static unsigned int g_id;
static int g_exit_status;

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

static unsigned int call1(u64 nr, u64 group)
{
    return (unsigned int)syscall1_s32(nr, group);
}

/* sys_spu_thread_get_exit_status: the return code, and the status it
 * wrote (0xFFFFFFFF when it wrote none) in *value. */
static unsigned int read_exit_status(sys_spu_thread_t thread, volatile unsigned int *value)
{
    unsigned int rc;
    g_exit_status = -1;
    rc = (unsigned int)syscall2_s32(SYS_SPU_THREAD_GET_EXIT_STATUS, thread,
                                    (u64)(unsigned long)&g_exit_status);
    *value = (unsigned int)g_exit_status;
    return rc;
}

/* Create a group of `type` with `num` threads; its id lands in g_id. */
static int create_typed(unsigned int type, unsigned int num)
{
    g_attr[0] = sizeof(GROUP_NAME);
    g_attr[1] = (unsigned int)(unsigned long)GROUP_NAME;
    g_attr[2] = type;
    g_attr[3] = 0;
    g_id = 0xFFFFFFFF;
    return syscall4_s32(SYS_SPU_THREAD_GROUP_CREATE,
                        (u64)(unsigned long)&g_id, num, 100,
                        (u64)(unsigned long)g_attr);
}

int main(void)
{
    sys_spu_group_t a, b;
    sys_spu_thread_t thread;
    unsigned int x, p;
    unsigned int cause, status;
    int ret;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1, ret);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 5;
    g_grpattr.name = "susp";
    ret = sysSpuThreadGroupCreate(&a, 1, 100, &g_grpattr);
    if (ret != 0) return fail(2, ret);
    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 5;
    g_thrattr.name = "susp";
    memset(&g_args, 0, sizeof(g_args));
    ret = sysSpuThreadInitialize(&thread, a, 0, &g_image, &g_thrattr, &g_args);
    if (ret != 0) return fail(3, ret);

    result_buf[2] = call1(SYS_SPU_THREAD_GROUP_SUSPEND, NO_SUCH_ID);
    result_buf[3] = call1(SYS_SPU_THREAD_GROUP_RESUME, NO_SUCH_ID);
    result_buf[4] = call1(SYS_SPU_THREAD_GROUP_YIELD, NO_SUCH_ID);
    result_buf[5] = call1(SYS_SPU_THREAD_GROUP_SUSPEND, a);
    result_buf[6] = call1(SYS_SPU_THREAD_GROUP_RESUME, a);
    result_buf[7] = call1(SYS_SPU_THREAD_GROUP_YIELD, a);

    ret = call1(SYS_SPU_THREAD_GROUP_START, a);
    if (ret != 0) return fail(4, ret);
    result_buf[8] = call1(SYS_SPU_THREAD_GROUP_YIELD, a);
    result_buf[9] = call1(SYS_SPU_THREAD_GROUP_RESUME, a);
    result_buf[10] = call1(SYS_SPU_THREAD_GROUP_SUSPEND, a);
    result_buf[11] = call1(SYS_SPU_THREAD_GROUP_SUSPEND, a);
    result_buf[12] = call1(SYS_SPU_THREAD_GROUP_YIELD, a);
    result_buf[13] = call1(SYS_SPU_THREAD_GROUP_RESUME, a);
    result_buf[14] = call1(SYS_SPU_THREAD_GROUP_RESUME, a);
    result_buf[15] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_GROUP_TERMINATE, a, 1);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(a, &cause, &status);
    if (ret != 0) return fail(5, ret);
    result_buf[16] = cause;
    result_buf[17] = status;

    result_buf[18] = call1(SYS_SPU_THREAD_GROUP_SUSPEND, a);
    result_buf[19] = call1(SYS_SPU_THREAD_GROUP_RESUME, a);
    result_buf[20] = call1(SYS_SPU_THREAD_GROUP_YIELD, a);
    result_buf[21] = call1(SYS_SPU_THREAD_GROUP_START, a);
    if (result_buf[21] == 0) {
        result_buf[22] = call1(SYS_SPU_THREAD_GROUP_SUSPEND, a);
        result_buf[23] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_GROUP_TERMINATE, a, 2);
        cause = status = 0xFFFFFFFF;
        ret = sysSpuThreadGroupJoin(a, &cause, &status);
        if (ret != 0) return fail(6, ret);
        result_buf[24] = cause;
        result_buf[25] = status;
    }
    ret = sysSpuThreadGroupDestroy(a);
    if (ret != 0) return fail(7, ret);

    ret = create_typed(0x18, 1);
    if (ret != 0) return fail(8, ret);
    x = g_id;
    ret = create_typed(0x20, 2);
    if (ret != 0) return fail(9, ret);
    p = g_id;
    result_buf[26] = call1(SYS_SPU_THREAD_GROUP_SUSPEND, x);
    result_buf[27] = call1(SYS_SPU_THREAD_GROUP_RESUME, x);
    result_buf[28] = call1(SYS_SPU_THREAD_GROUP_YIELD, x);
    result_buf[29] = call1(SYS_SPU_THREAD_GROUP_SUSPEND, p);
    result_buf[30] = call1(SYS_SPU_THREAD_GROUP_RESUME, p);
    result_buf[31] = call1(SYS_SPU_THREAD_GROUP_YIELD, p);
    ret = call1(SYS_SPU_THREAD_GROUP_DESTROY, p);
    if (ret != 0) return fail(10, ret);
    ret = call1(SYS_SPU_THREAD_GROUP_DESTROY, x);
    if (ret != 0) return fail(11, ret);

    ret = sysSpuThreadGroupCreate(&b, 1, 100, &g_grpattr);
    if (ret != 0) return fail(12, ret);
    g_args.arg0 = 1;
    ret = sysSpuThreadInitialize(&thread, b, 0, &g_image, &g_thrattr, &g_args);
    if (ret != 0) return fail(13, ret);
    ret = call1(SYS_SPU_THREAD_GROUP_START, b);
    if (ret != 0) return fail(14, ret);
    result_buf[32] = call1(SYS_SPU_THREAD_GROUP_SUSPEND, b);
    result_buf[33] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_WRITE_SPU_MB, thread, MESSAGE);
    result_buf[34] = call1(SYS_SPU_THREAD_GROUP_RESUME, b);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(b, &cause, &status);
    if (ret != 0) return fail(15, ret);
    result_buf[35] = cause;
    result_buf[36] = status;
    g_exit_status = -1;
    result_buf[37] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_GET_EXIT_STATUS, thread,
                                                (u64)(unsigned long)&g_exit_status);
    result_buf[38] = (unsigned int)g_exit_status;
    result_buf[39] = 0;

    /* B runs again after the join. Its thread waits on the mailbox, so
     * the first exit status read lands before the new run exits. */
    result_buf[40] = call1(SYS_SPU_THREAD_GROUP_START, b);
    if (result_buf[40] != 0) return fail(16, (int)result_buf[40]);
    result_buf[41] = read_exit_status(thread, &result_buf[42]);
    result_buf[43] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_WRITE_SPU_MB, thread, MESSAGE_2);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(b, &cause, &status);
    if (ret != 0) return fail(17, ret);
    result_buf[44] = cause;
    result_buf[45] = status;
    result_buf[46] = read_exit_status(thread, &result_buf[47]);

    /* B runs to its end and no join reads it before the next start. */
    ret = call1(SYS_SPU_THREAD_GROUP_START, b);
    if (ret != 0) return fail(18, ret);
    ret = syscall2_s32(SYS_SPU_THREAD_WRITE_SPU_MB, thread, MESSAGE_3);
    if (ret != 0) return fail(19, ret);
    (void)syscall1_s32(SYS_TIMER_USLEEP, SETTLE_US);
    result_buf[48] = call1(SYS_SPU_THREAD_GROUP_START, b);
    if (result_buf[48] == 0) {
        result_buf[49] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_WRITE_SPU_MB, thread,
                                                    MESSAGE_4);
        (void)syscall1_s32(SYS_TIMER_USLEEP, SETTLE_US);
    }
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(b, &cause, &status);
    if (ret != 0) return fail(20, ret);
    result_buf[50] = cause;
    result_buf[51] = status;
    result_buf[52] = read_exit_status(thread, &result_buf[53]);
    result_buf[54] = 0;
    result_buf[55] = (unsigned int)sysSpuThreadGroupDestroy(b);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
