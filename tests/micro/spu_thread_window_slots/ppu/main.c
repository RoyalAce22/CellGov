/* PPU program: every slot of the SPU thread window, and the join cause
 * and status each of three ways a thread group ends.
 *
 * Group 1 runs four threads of spu/main.c in MODE_WINDOW. Slot 0's
 * SPU_Sig_Notify_1 is set to OR mode before the start, so the three
 * other slots can each set one bit of it. Group 2 runs one thread that
 * calls sys_spu_thread_group_exit; group 3 runs one thread that yields
 * its group and then exits. The PPU joins each group in turn.
 *
 * Result layout (176 bytes, big-endian):
 *   +0:   u32 failed step (0 = none), u32 its return code,
 *         u32 group 1 join cause, u32 group 1 join status
 *   +16:  u32 group 2 join cause, u32 group 2 join status,
 *         u32 group 3 join cause, u32 group 3 join status
 *   +32:  u32[16] what slot 0 got through each slot's local-store alias,
 *         four words per slot: 0x5A000000 | slot << 16 | word
 *   +96:  per slot, u32 slot, u32 word, u32 0, u32 0: the bit mask slot
 *         0 collected (0xE), or the release word slot n got (0xB0 + n)
 *   +160: u32 the yield marker 0x59494C44, u32 0, u32 0, u32 0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 176
#define SLOTS        4

#define MODE_WINDOW     1
#define MODE_GROUP_EXIT 2
#define MODE_YIELD      3

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

static void write_frame(const void *data, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, data, len, &written);
    CGOV_OUT_FILE_WRITE(data, len);
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

/* Create a group of `count` threads in `mode`; the first thread's id
 * goes to *first. */
static int create_group(sys_spu_group_t *group, unsigned int count,
                        unsigned int mode, sys_spu_thread_t *first)
{
    unsigned int slot;
    int ret;

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "abiwind";
    ret = sysSpuThreadGroupCreate(group, count, 100, &g_grpattr);
    if (ret != 0)
        return ret;

    for (slot = 0; slot < count; slot++) {
        sys_spu_thread_t thread;
        memset(&g_thrattr, 0, sizeof(g_thrattr));
        g_thrattr.nsize = 8;
        g_thrattr.name = "abiwind";
        memset(&g_args, 0, sizeof(g_args));
        g_args.arg0 = (mode << 8) | slot;
        g_args.arg1 = (u64)(unsigned long)result_buf;
        ret = sysSpuThreadInitialize(&thread, *group, slot, &g_image,
                                     &g_thrattr, &g_args);
        if (ret != 0)
            return ret;
        if (slot == 0)
            *first = thread;
    }
    return 0;
}

/* Start `group`, join it, and store its cause and status at word `at`. */
static int run_group(sys_spu_group_t group, unsigned int at)
{
    unsigned int cause = 0xFFFFFFFF, status = 0xFFFFFFFF;
    int ret = sysSpuThreadGroupStart(group);
    if (ret != 0)
        return ret;
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0)
        return ret;
    result_buf[at] = cause;
    result_buf[at + 1] = status;
    return sysSpuThreadGroupDestroy(group);
}

int main(void)
{
    sys_spu_group_t group;
    sys_spu_thread_t first;
    int ret;

    memset(result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1, ret);

    ret = create_group(&group, SLOTS, MODE_WINDOW, &first);
    if (ret != 0) return fail(2, ret);
    ret = sysSpuThreadSetConfiguration(first, SPU_SIGNAL1_OR);
    if (ret != 0) return fail(3, ret);
    ret = run_group(group, 2);
    if (ret != 0) return fail(4, ret);

    ret = create_group(&group, 1, MODE_GROUP_EXIT, &first);
    if (ret != 0) return fail(5, ret);
    ret = run_group(group, 4);
    if (ret != 0) return fail(6, ret);

    ret = create_group(&group, 1, MODE_YIELD, &first);
    if (ret != 0) return fail(7, ret);
    ret = run_group(group, 6);
    if (ret != 0) return fail(8, ret);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
