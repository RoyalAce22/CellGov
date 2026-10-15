/* PPU program: what sys_spu_thread_group_terminate (177),
 * sys_spu_thread_get_exit_status (165) and sys_spu_thread_set_argument
 * (166) answer in each group state.
 *
 * Group A runs one thread of spu/main.c in MODE_PARK, which waits until
 * the group is terminated. Group B runs one thread in MODE_ARGS, which
 * puts its arg2 and arg3 at +96 and exits with EXIT_STATUS.
 * NO_SUCH_ID names no group and no thread.
 *
 * Result layout (128 bytes, big-endian u32 words):
 *   w0:  failed step (0 = none), w1: its return code
 *   A initialized, not started:
 *   w2:  terminate(NO_SUCH_ID)
 *   w3:  terminate(A)
 *   w4:  get_exit_status(NO_SUCH_ID, &status)
 *   w5:  get_exit_status(tA, &status)
 *   w6:  get_exit_status(tA, NULL)
 *   w7:  set_argument(NO_SUCH_ID, &new_args)
 *   w8:  set_argument(NO_SUCH_ID, NULL)
 *   w9:  set_argument(tA, NULL)
 *   A running:
 *   w10: set_argument(tA, &new_args)
 *   w11: get_exit_status(tA, &status)
 *   w12: terminate(A, TERMINATE_STATUS)
 *   w13: A join cause, w14: A join status
 *   A terminated:
 *   w15: get_exit_status(tA, &status)
 *   w16: terminate(A, 0)
 *   w17: set_argument(tA, &new_args)
 *   B:
 *   w18: set_argument(tB, &new_args) before the start
 *   w19: B join cause, w20: B join status
 *   w21: get_exit_status(tB, &status), w22: that status
 *   w23: get_exit_status(tB, NULL)
 *   w24-w27: the arg2 and arg3 B's thread started with, written by the
 *            SPU
 *   w28-w31: 0
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

#define MODE_PARK 1
#define MODE_ARGS 2

#define SYS_SPU_THREAD_GET_EXIT_STATUS  165
#define SYS_SPU_THREAD_SET_ARGUMENT     166
#define SYS_SPU_THREAD_GROUP_TERMINATE  177

#define NO_SUCH_ID       0xFFFFFFFFu
#define TERMINATE_STATUS 0xFFFFFFFEu
#define OLD_ARG2         0x1111111111111111ull
#define OLD_ARG3         0x2222222222222222ull
#define NEW_ARG2         0xA1A2A3A4A5A6A7A8ull
#define NEW_ARG3         0xB1B2B3B4B5B6B7B8ull

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
static sysSpuThreadArgument g_new_args;
static unsigned int g_status;

static unsigned int terminate(u64 group, u64 value)
{
    return (unsigned int)syscall2_s32(SYS_SPU_THREAD_GROUP_TERMINATE, group, value);
}

static unsigned int exit_status(u64 thread, u64 status_ptr)
{
    return (unsigned int)syscall2_s32(SYS_SPU_THREAD_GET_EXIT_STATUS, thread, status_ptr);
}

static unsigned int set_argument(u64 thread, u64 arg_ptr)
{
    return (unsigned int)syscall2_s32(SYS_SPU_THREAD_SET_ARGUMENT, thread, arg_ptr);
}

/* Create a one-thread group running spu/main.c in `mode`. */
static int create_group(sys_spu_group_t *group, unsigned int mode,
                        sys_spu_thread_t *thread)
{
    int ret;

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "grpctl";
    ret = sysSpuThreadGroupCreate(group, 1, 100, &g_grpattr);
    if (ret != 0)
        return ret;

    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 8;
    g_thrattr.name = "grpctl";
    memset(&g_args, 0, sizeof(g_args));
    g_args.arg0 = mode;
    g_args.arg1 = (u64)(unsigned long)result_buf;
    g_args.arg2 = OLD_ARG2;
    g_args.arg3 = OLD_ARG3;
    return sysSpuThreadInitialize(thread, *group, 0, &g_image, &g_thrattr, &g_args);
}

int main(void)
{
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    unsigned int cause, status;
    u64 status_ptr = (u64)(unsigned long)&g_status;
    u64 new_args_ptr = (u64)(unsigned long)&g_new_args;
    int ret;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;
    result_buf[28] = 0;
    result_buf[29] = 0;
    result_buf[30] = 0;
    result_buf[31] = 0;

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1, ret);

    /* Group A, initialized and not started. */
    ret = create_group(&group, MODE_PARK, &thread);
    if (ret != 0) return fail(2, ret);
    g_new_args.arg0 = MODE_PARK;
    g_new_args.arg1 = (u64)(unsigned long)result_buf;
    g_new_args.arg2 = NEW_ARG2;
    g_new_args.arg3 = NEW_ARG3;
    result_buf[2] = terminate(NO_SUCH_ID, 0);
    result_buf[3] = terminate(group, 0);
    result_buf[4] = exit_status(NO_SUCH_ID, status_ptr);
    result_buf[5] = exit_status(thread, status_ptr);
    result_buf[6] = exit_status(thread, 0);
    result_buf[7] = set_argument(NO_SUCH_ID, new_args_ptr);
    result_buf[8] = set_argument(NO_SUCH_ID, 0);
    result_buf[9] = set_argument(thread, 0);

    /* Group A running, then terminated. */
    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(3, ret);
    result_buf[10] = set_argument(thread, new_args_ptr);
    result_buf[11] = exit_status(thread, status_ptr);
    result_buf[12] = terminate(group, TERMINATE_STATUS);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(4, ret);
    result_buf[13] = cause;
    result_buf[14] = status;
    result_buf[15] = exit_status(thread, status_ptr);
    result_buf[16] = terminate(group, 0);
    result_buf[17] = set_argument(thread, new_args_ptr);
    ret = sysSpuThreadGroupDestroy(group);
    if (ret != 0) return fail(5, ret);

    /* Group B: new arguments before the start, then an exit status. */
    ret = create_group(&group, MODE_ARGS, &thread);
    if (ret != 0) return fail(6, ret);
    g_new_args.arg0 = MODE_ARGS;
    result_buf[18] = set_argument(thread, new_args_ptr);
    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(7, ret);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(8, ret);
    result_buf[19] = cause;
    result_buf[20] = status;
    g_status = 0xFFFFFFFF;
    result_buf[21] = exit_status(thread, status_ptr);
    result_buf[22] = g_status;
    result_buf[23] = exit_status(thread, 0);
    ret = sysSpuThreadGroupDestroy(group);
    if (ret != 0) return fail(9, ret);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
