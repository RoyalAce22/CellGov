/* PPU program: the root-only SPU thread-group types
 * sys_spu_thread_group_create (170) takes. The build signs this probe's
 * EBOOT with control_flags1 0, as a retail title is signed, so the
 * console answers as it does for a game. spu_group_root_types asks the
 * same questions with root permission.
 *
 * No group is ever initialized or started; each created group is
 * destroyed before the probe ends.
 *
 * Result layout (64 bytes, big-endian u32 words; each word is the call's
 * return code):
 *   w0:  failed step (0 = none), w1: its return code
 *   w2:  create type 0x20A num 1
 *   w3:  create type 0x20A num 2
 *   w4:  create type 0x002 num 1
 *   w5:  create type 0x00A num 2
 *   w6:  create type 0x022 num 2
 *   w7:  create type 0x102 num 1
 *   w8:  create type 0x30A num 1, a type no group takes
 *   w9-w15: 0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 64

#define SYS_SPU_THREAD_GROUP_CREATE     170
#define SYS_SPU_THREAD_GROUP_DESTROY    171

#define MAX_LIVE 8

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

/* sys_spu_thread_group_attribute: nsize, name, type, option. */
static unsigned int g_attr[4] __attribute__((aligned(16)));
static const char GROUP_NAME[] = "root";

static unsigned int g_ids[MAX_LIVE];
static unsigned int g_live;
static unsigned int g_id;

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

/* Create a group; a created group is kept for destroy_all. */
static unsigned int create(unsigned int type, unsigned int num)
{
    int ret;

    g_attr[0] = sizeof(GROUP_NAME);
    g_attr[1] = (unsigned int)(unsigned long)GROUP_NAME;
    g_attr[2] = type;
    g_attr[3] = 0;
    g_id = 0xFFFFFFFF;
    ret = syscall4_s32(SYS_SPU_THREAD_GROUP_CREATE,
                       (u64)(unsigned long)&g_id, num, 100,
                       (u64)(unsigned long)g_attr);
    if (ret == 0 && g_live < MAX_LIVE)
        g_ids[g_live++] = g_id;
    return (unsigned int)ret;
}

/* Destroy every kept group; returns the first failure. */
static int destroy_all(void)
{
    int ret, first = 0;

    while (g_live > 0) {
        ret = syscall1_s32(SYS_SPU_THREAD_GROUP_DESTROY, g_ids[--g_live]);
        if (ret != 0 && first == 0)
            first = ret;
    }
    return first;
}

int main(void)
{
    int ret;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;
    for (ret = 9; ret < RESULT_BYTES / 4; ret++)
        result_buf[ret] = 0;
    g_live = 0;

    result_buf[2] = create(0x20A, 1);
    result_buf[3] = create(0x20A, 2);
    result_buf[4] = create(0x002, 1);
    result_buf[5] = create(0x00A, 2);
    result_buf[6] = create(0x022, 2);
    result_buf[7] = create(0x102, 1);
    result_buf[8] = create(0x30A, 1);
    ret = destroy_all();
    if (ret != 0) return fail(1, ret);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
