/* PPU program: the SPU pool limits sys_spu_initialize (169) sets and
 * the refusals sys_spu_thread_group_create (170) answers against them.
 *
 * No group is ever initialized or started; each is destroyed before
 * the phase that follows. The last call restores the default limits
 * (no Raw SPU).
 *
 * Group types: CONTEXT 0x00 (normal), EXCLUSIVE 0x18 (exclusive
 * non-context), COOP 0x20 (cooperate with system).
 *
 * Result layout (128 bytes, big-endian u32 words; each word is the call's
 * return code):
 *   w0:  failed step (0 = none), w1: its return code
 *   default limits:
 *   w2:  initialize(6, 6)
 *   w3:  initialize(0, 0)
 *   w4:  create CONTEXT num 6
 *   w5:  create CONTEXT num 6, the first still alive
 *   w6:  create CONTEXT num 7
 *   w7:  create EXCLUSIVE num 5, both CONTEXT groups alive
 *   w8:  create EXCLUSIVE num 1
 *   w9:  initialize(6, 1) with all three groups alive
 *   after destroying them:
 *   w10: create EXCLUSIVE num 6
 *   w11: create EXCLUSIVE num 7
 *   w12: initialize(6, 1)
 *   w13: create CONTEXT num 6
 *   w14: create CONTEXT num 5
 *   w15: create EXCLUSIVE num 4, the CONTEXT group alive
 *   w16: create EXCLUSIVE num 1
 *   w17: initialize(6, 2) with the two groups alive
 *   w18: initialize(6, 0) with the two groups alive
 *   after destroying them:
 *   w19: create COOP num 1
 *   w20: create COOP num 2
 *   w21: create COOP num 2, the first still alive
 *   after destroying it:
 *   w22: initialize(6, 0)
 *   w23: initialize(6, 1)
 *   w24: create EXCLUSIVE num 6, more than the five SPUs left
 *   w25: initialize(6, 0)
 *   w26: create COOP num 7
 *   w27: create type 0x01 num 1, a type no group takes
 *   after destroying any created group:
 *   w28: initialize(6, 0)
 *   w29-w31: 0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 128

#define SYS_SPU_INITIALIZE              169
#define SYS_SPU_THREAD_GROUP_CREATE     170
#define SYS_SPU_THREAD_GROUP_DESTROY    171

#define CONTEXT   0x00u
#define EXCLUSIVE 0x18u
#define COOP      0x20u
#define BAD_TYPE  0x01u

#define MAX_LIVE 4

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

/* sys_spu_thread_group_attribute: nsize, name, type, option. */
static unsigned int g_attr[4] __attribute__((aligned(16)));
static const char GROUP_NAME[] = "limits";

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

static unsigned int initialize(u64 usable, u64 raw)
{
    return (unsigned int)syscall2_s32(SYS_SPU_INITIALIZE, usable, raw);
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
    result_buf[29] = 0;
    result_buf[30] = 0;
    result_buf[31] = 0;
    g_live = 0;

    result_buf[2] = initialize(6, 6);
    result_buf[3] = initialize(0, 0);
    result_buf[4] = create(CONTEXT, 6);
    result_buf[5] = create(CONTEXT, 6);
    result_buf[6] = create(CONTEXT, 7);
    result_buf[7] = create(EXCLUSIVE, 5);
    result_buf[8] = create(EXCLUSIVE, 1);
    result_buf[9] = initialize(6, 1);
    ret = destroy_all();
    if (ret != 0) return fail(1, ret);

    result_buf[10] = create(EXCLUSIVE, 6);
    result_buf[11] = create(EXCLUSIVE, 7);
    result_buf[12] = initialize(6, 1);
    result_buf[13] = create(CONTEXT, 6);
    result_buf[14] = create(CONTEXT, 5);
    result_buf[15] = create(EXCLUSIVE, 4);
    result_buf[16] = create(EXCLUSIVE, 1);
    result_buf[17] = initialize(6, 2);
    result_buf[18] = initialize(6, 0);
    ret = destroy_all();
    if (ret != 0) return fail(2, ret);

    result_buf[19] = create(COOP, 1);
    result_buf[20] = create(COOP, 2);
    result_buf[21] = create(COOP, 2);
    ret = destroy_all();
    if (ret != 0) return fail(3, ret);

    result_buf[22] = initialize(6, 0);
    result_buf[23] = initialize(6, 1);
    result_buf[24] = create(EXCLUSIVE, 6);
    ret = destroy_all();
    if (ret != 0) return fail(4, ret);

    result_buf[25] = initialize(6, 0);
    result_buf[26] = create(COOP, 7);
    result_buf[27] = create(BAD_TYPE, 1);
    ret = destroy_all();
    if (ret != 0) return fail(5, ret);

    result_buf[28] = initialize(6, 0);
    if (result_buf[28] != 0) return fail(6, (int)result_buf[28]);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
