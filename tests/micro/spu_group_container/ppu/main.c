/* PPU program: the memory an SPU thread group of type
 * MEMORY_FROM_CONTAINER (0x04) takes from the memory container its
 * attribute names, and what sys_spu_thread_group_create (170) answers
 * when the container is short.
 *
 * No group is ever initialized or started; each is destroyed before
 * the probe ends. The two containers stay until the process exits.
 *
 * Containers: A from sys_memory_container_create(1 MiB), B from
 * sys_memory_container_create(1.5 MiB).
 *
 * Result layout (128 bytes, big-endian u32 words; each word is the
 * call's return code unless named otherwise):
 *   w0:  failed step (0 = none), w1: its return code
 *   w2:  initialize(6, 0)
 *   w3:  container_create(1 MiB), w4: id A
 *   w5:  container_create(1.5 MiB), w6: id B
 *   w7:  create in B num 5
 *   w8:  create in B num 4
 *   w9:  destroy that group
 *   w10: create in A num 4 (kept)
 *   w11: create in A num 1
 *   w12: create CONTEXT num 2 (kept; the pool is now full)
 *   w13: create in A num 1, the pool full and A spent
 *   w14: create in B num 1, the pool full
 *   w15: destroy the group in A
 *   w16: create in A num 1
 *   w17: destroy that group
 *   w18: destroy the CONTEXT group
 *   w19: create in a container id no create minted, num 1
 *   w20: container_create(512 KiB)
 *   w21: initialize(6, 0)
 *   w22-w31: 0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 128

#define SYS_MEMORY_CONTAINER_CREATE     341
#define SYS_SPU_INITIALIZE              169
#define SYS_SPU_THREAD_GROUP_CREATE     170
#define SYS_SPU_THREAD_GROUP_DESTROY    171

#define CONTEXT   0x00u
#define CONTAINER 0x04u

#define NO_SUCH_CONTAINER 0x7FFFFFF0u

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

/* sys_spu_thread_group_attribute: nsize, name, type, option. */
static unsigned int g_attr[4] __attribute__((aligned(16)));
static const char GROUP_NAME[] = "container";

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

static unsigned int container_create(u64 size)
{
    g_id = 0xEEEEEEEE;
    return (unsigned int)syscall2_s32(SYS_MEMORY_CONTAINER_CREATE,
                                      (u64)(unsigned long)&g_id, size);
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

static unsigned int destroy(unsigned int id)
{
    return (unsigned int)syscall1_s32(SYS_SPU_THREAD_GROUP_DESTROY, id);
}

/* Destroy a group a refused step may have left behind. */
static void drop(unsigned int id)
{
    if (id != 0xFFFFFFFFu)
        destroy(id);
}

int main(void)
{
    unsigned int a, b, in_a, normal, tmp, i;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;
    for (i = 22; i < RESULT_BYTES / 4; i++)
        result_buf[i] = 0;

    result_buf[2] = (unsigned int)syscall2_s32(SYS_SPU_INITIALIZE, 6, 0);
    result_buf[3] = container_create(0x100000);
    result_buf[4] = g_id;
    if (result_buf[3] != 0) return fail(1, (int)result_buf[3]);
    a = g_id;
    result_buf[5] = container_create(0x180000);
    result_buf[6] = g_id;
    if (result_buf[5] != 0) return fail(2, (int)result_buf[5]);
    b = g_id;

    result_buf[7] = create(CONTAINER, b, 5, &tmp);
    drop(tmp);
    result_buf[8] = create(CONTAINER, b, 4, &tmp);
    if (result_buf[8] != 0) return fail(3, (int)result_buf[8]);
    result_buf[9] = destroy(tmp);

    result_buf[10] = create(CONTAINER, a, 4, &in_a);
    if (result_buf[10] != 0) return fail(4, (int)result_buf[10]);
    result_buf[11] = create(CONTAINER, a, 1, &tmp);
    drop(tmp);
    result_buf[12] = create(CONTEXT, 0, 2, &normal);
    if (result_buf[12] != 0) {
        destroy(in_a);
        return fail(5, (int)result_buf[12]);
    }
    result_buf[13] = create(CONTAINER, a, 1, &tmp);
    drop(tmp);
    result_buf[14] = create(CONTAINER, b, 1, &tmp);
    drop(tmp);

    result_buf[15] = destroy(in_a);
    result_buf[16] = create(CONTAINER, a, 1, &tmp);
    result_buf[17] = tmp != 0xFFFFFFFFu ? destroy(tmp) : 0xEEEEEEEEu;
    result_buf[18] = destroy(normal);

    result_buf[19] = create(CONTAINER, NO_SUCH_CONTAINER, 1, &tmp);
    drop(tmp);
    result_buf[20] = container_create(0x80000);
    result_buf[21] = (unsigned int)syscall2_s32(SYS_SPU_INITIALIZE, 6, 0);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
