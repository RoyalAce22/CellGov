/* PPU program: the edges of the SPU pool sys_spu_thread_group_create
 * (170) answers against, beyond tests/micro/spu_group_limits: an
 * unreadable attribute, the two memory-container group types, and
 * whether a finished group still holds its SPUs.
 *
 * Group types: EXCLUSIVE 0x18 (exclusive non-context), CONTAINER 0x04
 * (memory from container), COOP_CONTAINER 0x24 (cooperate with system,
 * memory from container). NO_SUCH_ID names no memory container.
 *
 * Result layout (64 bytes, big-endian u32 words; each word is the call's
 * return code unless named otherwise):
 *   w0:  failed step (0 = none), w1: its return code
 *   w2:  create num 1, attribute NULL
 *   w3:  create num 0, attribute NULL
 *   w4:  create CONTAINER num 1, container NO_SUCH_ID
 *   w5:  create COOP_CONTAINER num 2, container NO_SUCH_ID
 *   after destroying any created group:
 *   w6:  initialize(6, 4), leaving two SPUs for SPU threads
 *   w7:  create EXCLUSIVE num 1
 *   w8:  that group's join cause, after its one thread exits
 *   w9:  create EXCLUSIVE num 1, the finished group not destroyed
 *   after destroying both:
 *   w10: initialize(6, 0)
 *   w11-w15: 0
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

#define SYS_SPU_INITIALIZE              169
#define SYS_SPU_THREAD_GROUP_CREATE     170
#define SYS_SPU_THREAD_GROUP_DESTROY    171

#define EXCLUSIVE      0x18u
#define CONTAINER      0x04u
#define COOP_CONTAINER 0x24u
#define NO_SUCH_ID     0xFFFFFFFFu

#define MAX_LIVE 4

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

/* sys_spu_thread_group_attribute: nsize, name, type, container id. */
static unsigned int g_attr[4] __attribute__((aligned(16)));
static const char GROUP_NAME[] = "edges";

static unsigned int g_ids[MAX_LIVE];
static unsigned int g_live;
static unsigned int g_id;

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

static sysSpuImage g_image;
static sysSpuThreadAttribute g_thrattr;
static sysSpuThreadArgument g_args;

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

static unsigned int initialize(u64 usable, u64 raw)
{
    return (unsigned int)syscall2_s32(SYS_SPU_INITIALIZE, usable, raw);
}

/* Create a group with the attribute at `attr`; a created group is kept
 * for destroy_all. */
static unsigned int create_at(u64 attr, unsigned int num)
{
    int ret;

    g_id = 0xFFFFFFFF;
    ret = syscall4_s32(SYS_SPU_THREAD_GROUP_CREATE,
                       (u64)(unsigned long)&g_id, num, 100, attr);
    if (ret == 0 && g_live < MAX_LIVE)
        g_ids[g_live++] = g_id;
    return (unsigned int)ret;
}

static unsigned int create(unsigned int type, unsigned int num)
{
    g_attr[0] = sizeof(GROUP_NAME);
    g_attr[1] = (unsigned int)(unsigned long)GROUP_NAME;
    g_attr[2] = type;
    g_attr[3] = NO_SUCH_ID;
    return create_at((u64)(unsigned long)g_attr, num);
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
    sys_spu_thread_t thread;
    unsigned int cause, status;
    int ret;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;
    for (ret = 11; ret < RESULT_BYTES / 4; ret++)
        result_buf[ret] = 0;
    g_live = 0;

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1, ret);

    result_buf[2] = create_at(0, 1);
    result_buf[3] = create_at(0, 0);
    result_buf[4] = create(CONTAINER, 1);
    result_buf[5] = create(COOP_CONTAINER, 2);
    ret = destroy_all();
    if (ret != 0) return fail(2, ret);

    result_buf[6] = initialize(6, 4);
    result_buf[7] = create(EXCLUSIVE, 1);
    if (result_buf[7] != 0) return fail(3, (int)result_buf[7]);
    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 6;
    g_thrattr.name = "edges";
    memset(&g_args, 0, sizeof(g_args));
    ret = sysSpuThreadInitialize(&thread, g_id, 0, &g_image, &g_thrattr, &g_args);
    if (ret != 0) return fail(4, ret);
    ret = sysSpuThreadGroupStart(g_id);
    if (ret != 0) return fail(5, ret);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(g_id, &cause, &status);
    if (ret != 0) return fail(6, ret);
    result_buf[8] = cause;
    result_buf[9] = create(EXCLUSIVE, 1);
    ret = destroy_all();
    if (ret != 0) return fail(7, ret);

    result_buf[10] = initialize(6, 0);
    if (result_buf[10] != 0) return fail(8, (int)result_buf[10]);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
