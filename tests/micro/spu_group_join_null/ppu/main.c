/* PPU program: what sys_spu_thread_group_join (178) answers, and what
 * it writes, when its cause or status pointer is NULL.
 *
 * Each case runs one one-thread group of spu/main.c, which spins for
 * SPIN_ITERATIONS and then calls sys_spu_thread_group_exit(EXIT_STATUS),
 * so the probed join is issued while the group still runs. After the
 * probed join the group is started again: a start that succeeds shows
 * the probed join waited for the group to end, and a start the group
 * refuses shows the probed join returned while the group ran. A
 * follow-up join with both pointers then waits for whichever run is
 * live, so no case can leave a running group behind.
 *
 * Cases, in order:
 *   0: join(&cause, &status)    (the control)
 *   1: join(NULL,   &status)
 *   2: join(&cause, NULL)
 *   3: join(NULL,   NULL)
 *
 * Result layout (160 bytes, big-endian u32 words):
 *   w0: failed step (0 = none), w1: its return code
 *   case k at w(2 + 8k):
 *     +0 probed join rc
 *     +1 cause after it (pre-filled SENTINEL)
 *     +2 status after it (pre-filled SENTINEL)
 *     +3 rc of the second start
 *     +4 follow-up join rc, +5 its cause, +6 its status
 *     +7 destroy rc
 *   w34-w39: 0
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

#define RESULT_BYTES 160
#define CASES        4
#define CASE_WORDS   8

#define SYS_SPU_THREAD_GROUP_JOIN 178

#define SENTINEL 0xFFFFFFFFu

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
static unsigned int g_cause;
static unsigned int g_status;

static unsigned int join(u64 group, u64 cause_ptr, u64 status_ptr)
{
    return (unsigned int)syscall3_s32(SYS_SPU_THREAD_GROUP_JOIN, group, cause_ptr, status_ptr);
}

static int create_group(sys_spu_group_t *group)
{
    sys_spu_thread_t thread;
    int ret;

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "joinnul";
    ret = sysSpuThreadGroupCreate(group, 1, 100, &g_grpattr);
    if (ret != 0)
        return ret;

    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 8;
    g_thrattr.name = "joinnul";
    memset(&g_args, 0, sizeof(g_args));
    return sysSpuThreadInitialize(&thread, *group, 0, &g_image, &g_thrattr, &g_args);
}

int main(void)
{
    sys_spu_group_t group;
    u64 cause_ptr = (u64)(unsigned long)&g_cause;
    u64 status_ptr = (u64)(unsigned long)&g_status;
    unsigned int k;
    int ret;

    memset((void *)result_buf, 0, sizeof(result_buf));

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1, ret);

    for (k = 0; k < CASES; k++) {
        volatile unsigned int *w = &result_buf[2 + CASE_WORDS * k];
        u64 probe_cause = (k & 1) ? 0 : cause_ptr;
        u64 probe_status = (k & 2) ? 0 : status_ptr;

        ret = create_group(&group);
        if (ret != 0) return fail(2 + 2 * k, ret);
        ret = sysSpuThreadGroupStart(group);
        if (ret != 0) return fail(3 + 2 * k, ret);

        g_cause = SENTINEL;
        g_status = SENTINEL;
        w[0] = join(group, probe_cause, probe_status);
        w[1] = g_cause;
        w[2] = g_status;

        w[3] = (unsigned int)sysSpuThreadGroupStart(group);

        g_cause = SENTINEL;
        g_status = SENTINEL;
        w[4] = join(group, cause_ptr, status_ptr);
        w[5] = g_cause;
        w[6] = g_status;

        w[7] = (unsigned int)sysSpuThreadGroupDestroy(group);
    }

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
