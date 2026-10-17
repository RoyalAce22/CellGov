/* PPU program: the priority sys_spu_thread_group_create (170) accepts,
 * and what sys_spu_thread_group_set_priority (179) and
 * sys_spu_thread_group_get_priority (180) answer for each group type.
 * The build signs this probe's EBOOT with control_flags1 0, as a retail
 * title is signed; spu_group_priority asks the same questions with root
 * permission.
 *
 * No group is ever initialized or started. Group types: CONTEXT 0x00
 * (normal), EXCLUSIVE 0x18 (exclusive non-context), COOP 0x20
 * (cooperate with system). NO_SUCH_ID names no group. A word named
 * "read back" is the priority get_priority wrote; every other word is
 * the call's return code.
 *
 * Result layout (128 bytes, big-endian u32 words):
 *   w0:  failed step (0 = none), w1: its return code
 *   create:
 *   w2:  CONTEXT priority 15
 *   w3:  CONTEXT priority 16 (group C)
 *   w4:  CONTEXT priority 255 (group D)
 *   w5:  CONTEXT priority 256
 *   w6:  CONTEXT priority -1
 *   w7:  EXCLUSIVE priority 0 (group X)
 *   w8:  COOP num 2 priority 0 (group P)
 *   set and get on C:
 *   w9:  get(C), w10: read back
 *   w11: set(C, 200), w12: get(C), w13: read back
 *   w14: set(C, 15), w15: set(C, 256), w16: set(C, -1)
 *   unknown group and NULL out-pointer:
 *   w17: set(NO_SUCH_ID, 100), w18: set(NO_SUCH_ID, 15)
 *   w19: get(NO_SUCH_ID)
 *   w20: get(C, NULL)
 *   the other types:
 *   w21: set(X, 100), w22: get(X), w23: read back
 *   w24: set(P, 100), w25: get(P), w26: read back
 *   w27: get(D), w28: read back
 *   w29-w31: 0
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 128

#define SYS_SPU_THREAD_GROUP_CREATE       170
#define SYS_SPU_THREAD_GROUP_DESTROY      171
#define SYS_SPU_THREAD_GROUP_SET_PRIORITY 179
#define SYS_SPU_THREAD_GROUP_GET_PRIORITY 180

#define CONTEXT    0x00u
#define EXCLUSIVE  0x18u
#define COOP       0x20u
#define NO_SUCH_ID 0xFFFFFFFFu

#define MAX_LIVE 8

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

/* sys_spu_thread_group_attribute: nsize, name, type, option. */
static unsigned int g_attr[4] __attribute__((aligned(16)));
static const char GROUP_NAME[] = "prio";

static unsigned int g_ids[MAX_LIVE];
static unsigned int g_live;
static unsigned int g_id;
static int g_prio;

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

/* Create a group; a created group is kept for destroy_all, and its id
 * left in g_id. */
static unsigned int create(unsigned int type, unsigned int num, int prio)
{
    int ret;

    g_attr[0] = sizeof(GROUP_NAME);
    g_attr[1] = (unsigned int)(unsigned long)GROUP_NAME;
    g_attr[2] = type;
    g_attr[3] = 0;
    g_id = 0xFFFFFFFF;
    ret = syscall4_s32(SYS_SPU_THREAD_GROUP_CREATE,
                       (u64)(unsigned long)&g_id, num, (u64)(s64)prio,
                       (u64)(unsigned long)g_attr);
    if (ret == 0 && g_live < MAX_LIVE)
        g_ids[g_live++] = g_id;
    return (unsigned int)ret;
}

static unsigned int set_prio(u64 group, int prio)
{
    return (unsigned int)syscall2_s32(SYS_SPU_THREAD_GROUP_SET_PRIORITY,
                                      group, (u64)(s64)prio);
}

/* get_priority into g_prio; the read-back word goes to `word`. */
static unsigned int get_prio(u64 group, unsigned int word)
{
    unsigned int ret;

    g_prio = (int)0xEEEEEEEE;
    ret = (unsigned int)syscall2_s32(SYS_SPU_THREAD_GROUP_GET_PRIORITY,
                                     group, (u64)(unsigned long)&g_prio);
    result_buf[word] = (unsigned int)g_prio;
    return ret;
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
    unsigned int c, d, x, p;
    int ret;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;
    result_buf[29] = 0;
    result_buf[30] = 0;
    result_buf[31] = 0;
    g_live = 0;

    result_buf[2] = create(CONTEXT, 1, 15);
    result_buf[3] = create(CONTEXT, 1, 16);
    if (result_buf[3] != 0) return fail(1, (int)result_buf[3]);
    c = g_id;
    result_buf[4] = create(CONTEXT, 1, 255);
    if (result_buf[4] != 0) return fail(2, (int)result_buf[4]);
    d = g_id;
    result_buf[5] = create(CONTEXT, 1, 256);
    result_buf[6] = create(CONTEXT, 1, -1);
    result_buf[7] = create(EXCLUSIVE, 1, 0);
    if (result_buf[7] != 0) return fail(3, (int)result_buf[7]);
    x = g_id;
    result_buf[8] = create(COOP, 2, 0);
    if (result_buf[8] != 0) return fail(4, (int)result_buf[8]);
    p = g_id;

    result_buf[9] = get_prio(c, 10);
    result_buf[11] = set_prio(c, 200);
    result_buf[12] = get_prio(c, 13);
    result_buf[14] = set_prio(c, 15);
    result_buf[15] = set_prio(c, 256);
    result_buf[16] = set_prio(c, -1);

    result_buf[17] = set_prio(NO_SUCH_ID, 100);
    result_buf[18] = set_prio(NO_SUCH_ID, 15);
    g_prio = 0;
    result_buf[19] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_GROUP_GET_PRIORITY,
                                                NO_SUCH_ID, (u64)(unsigned long)&g_prio);
    result_buf[20] = (unsigned int)syscall2_s32(SYS_SPU_THREAD_GROUP_GET_PRIORITY, c, 0);

    result_buf[21] = set_prio(x, 100);
    result_buf[22] = get_prio(x, 23);
    result_buf[24] = set_prio(p, 100);
    result_buf[25] = get_prio(p, 26);
    result_buf[27] = get_prio(d, 28);

    ret = destroy_all();
    if (ret != 0) return fail(5, ret);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
