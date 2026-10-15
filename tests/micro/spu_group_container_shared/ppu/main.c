/* PPU program: what a cooperating container-type SPU thread group
 * (0x24) takes from its memory container, and whether an allocation
 * from a container leaves less of it for a group.
 *
 * No group is ever initialized or started; each is destroyed before
 * the probe ends. The containers and the allocation stay until the
 * process exits.
 *
 * Result layout (64 bytes, big-endian u32 words; each word is the
 * call's return code unless named otherwise):
 *   w0:  failed step (0 = none), w1: its return code
 *   w2:  initialize(6, 0)
 *   w3:  container_create(1 MiB), w4: id A
 *   w5:  create COOP|CONTAINER in A num 4 (kept)
 *   w6:  create CONTAINER in A num 3, the cooperating group alive
 *   w7:  destroy that group (0xEEEEEEEE when w6 created none)
 *   w8:  destroy the cooperating group (0xEEEEEEEE when w5 created none)
 *   w9:  container_create(1 MiB), w10: id C
 *   w11: allocate_from_container(1 MiB, C, 1 MiB pages), w12: its address
 *   w13: create CONTAINER in C num 1
 *   w14: destroy that group (0xEEEEEEEE when w13 created none)
 *   w15: initialize(6, 0)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 64

#define SYS_MEMORY_ALLOCATE_FROM_CONTAINER 350
#define SYS_MEMORY_CONTAINER_CREATE        341
#define SYS_SPU_INITIALIZE                 169
#define SYS_SPU_THREAD_GROUP_CREATE        170
#define SYS_SPU_THREAD_GROUP_DESTROY       171

#define CONTAINER      0x04u
#define COOP_CONTAINER 0x24u
#define PAGE_SIZE_1M   0x400u

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

/* sys_spu_thread_group_attribute: nsize, name, type, option. */
static unsigned int g_attr[4] __attribute__((aligned(16)));
static const char GROUP_NAME[] = "shared";

static volatile unsigned int g_id;

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

static unsigned int container_create(void)
{
    g_id = 0xEEEEEEEE;
    return (unsigned int)syscall2_s32(SYS_MEMORY_CONTAINER_CREATE,
                                      (u64)(unsigned long)&g_id, 0x100000);
}

/* Create a group; on success *id holds it, otherwise 0xFFFFFFFF. */
static unsigned int create(unsigned int type, unsigned int ct, unsigned int num,
                           unsigned int *id)
{
    int ret;

    g_attr[0] = sizeof(GROUP_NAME);
    g_attr[1] = (unsigned int)(unsigned long)GROUP_NAME;
    g_attr[2] = type;
    g_attr[3] = ct;
    g_id = 0xFFFFFFFF;
    ret = syscall4_s32(SYS_SPU_THREAD_GROUP_CREATE,
                       (u64)(unsigned long)&g_id, num, 100,
                       (u64)(unsigned long)g_attr);
    *id = ret == 0 ? g_id : 0xFFFFFFFFu;
    return (unsigned int)ret;
}

/* Destroy a created group; 0xEEEEEEEE when the create made none. */
static unsigned int destroy(unsigned int id)
{
    if (id == 0xFFFFFFFFu)
        return 0xEEEEEEEEu;
    return (unsigned int)syscall1_s32(SYS_SPU_THREAD_GROUP_DESTROY, id);
}

int main(void)
{
    unsigned int a, c, coop, tmp;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;

    result_buf[2] = (unsigned int)syscall2_s32(SYS_SPU_INITIALIZE, 6, 0);
    result_buf[3] = container_create();
    result_buf[4] = g_id;
    if (result_buf[3] != 0) return fail(1, (int)result_buf[3]);
    a = g_id;

    result_buf[5] = create(COOP_CONTAINER, a, 4, &coop);
    result_buf[6] = create(CONTAINER, a, 3, &tmp);
    result_buf[7] = destroy(tmp);
    result_buf[8] = destroy(coop);

    result_buf[9] = container_create();
    result_buf[10] = g_id;
    if (result_buf[9] != 0) return fail(2, (int)result_buf[9]);
    c = g_id;
    g_id = 0xEEEEEEEE;
    result_buf[11] = (unsigned int)syscall4_s32(SYS_MEMORY_ALLOCATE_FROM_CONTAINER,
                                                0x100000, c, PAGE_SIZE_1M,
                                                (u64)(unsigned long)&g_id);
    result_buf[12] = g_id;
    result_buf[13] = create(CONTAINER, c, 1, &tmp);
    result_buf[14] = destroy(tmp);
    result_buf[15] = (unsigned int)syscall2_s32(SYS_SPU_INITIALIZE, 6, 0);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
