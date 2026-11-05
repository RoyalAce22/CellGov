/* PPU program: an SPU thread's 32-byte get across the seam between two
 * 64 KiB shared-memory segments mapped end to end.
 *
 * The probe allocates one 256 MiB area A of 64 KiB pages (330) and two
 * keyless 64 KiB segments M0 and M1 (332), and maps M0 at A and M1 at
 * A + 64K (334). E is A + 64K, the seam. The main thread fills the 64
 * bytes from E - 48 with 0x10, 0x11, ... 0x4F by aligned word stores.
 *
 * Cases (the get's effective address):
 *   0  E - 48, all 32 bytes in M0 (the control)
 *   1  E - 16, 16 bytes in M0 and 16 in M1
 *
 * Per case: create a one-thread group and its thread, connect the PPU
 * event queue to the group's exception event, start the group, then
 * receive one event (2 s timeout). The SPU program gets the 32 bytes and
 * puts them to the case's landing buffer. A received event means the
 * MFC refused the get, and the group is terminated. Then join the
 * group, disconnect the event and destroy the group.
 *
 * Result frame (192 bytes, big-endian u32 words):
 *    0  the failed step (case << 8 | step, 0 = none)
 *    1  its return code
 *    2  CASE_COUNT
 *    3  the queue destroy's answer
 *    4  330 rc     5  A (low word)
 *    6  332 M0 rc  7  332 M1 rc
 *    8  334 M0 at A    9  334 M1 at A + 64K
 *   10  335 A + 64K   11  335 A
 *   12  329 M1        13  329 M0
 *   14  331 A         15  zero
 *   16 + 16k, case k:
 *       +0 receive's answer, +1 data2 low (the cause) of the event,
 *       +2 terminate's answer (SKIP when no event came),
 *       +3 join's answer, +4 join cause, +5 join status,
 *       +6 disconnect's answer, +7 destroy's answer,
 *       +8..+15 the eight landing words (0xEEEEEEEE preset)
 *
 * SKIP is 0xDEADD00D.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <sys/event_queue.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MMAPPER_FREE_SHM         329
#define SYS_MMAPPER_ALLOCATE_ADDRESS 330
#define SYS_MMAPPER_FREE_ADDRESS     331
#define SYS_MMAPPER_ALLOCATE_SHM     332
#define SYS_MMAPPER_MAP_SHM          334
#define SYS_MMAPPER_UNMAP_SHM        335

#define KEYLESS   0xFFFF000000000000ULL
#define FLAG_64K  0x200
#define PROT_RW   0x40000
#define AREA      0x10000000ULL
#define K64       0x10000ULL

#define CASE_COUNT 2
#define CASE_WORDS 16
#define WORDS (16 + CASE_WORDS * CASE_COUNT)
#define EVENT_EXCEPTION 2
#define RECEIVE_USEC 2000000ull
#define TERMINATE_STATUS 0x7E57
#define SKIP 0xDEADD00Du

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };
static const unsigned long long OFFSET[CASE_COUNT] = { 48, 16 };

static volatile unsigned int result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int landing[CASE_COUNT][32] __attribute__((aligned(128)));
static volatile unsigned int w32 __attribute__((aligned(8)));

#define EA(p) ((u64)(unsigned long)(const void *)(p))

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
}

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = sizeof(result);
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)result, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)result, len);
}

static u64 unmap(u64 addr)
{
    w32 = 0xFFFFFFFFu;
    return sc(SYS_MMAPPER_UNMAP_SHM, addr, EA(&w32), 0, 0);
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

static sysSpuImage image;
static sysSpuThreadGroupAttribute grpattr;
static sysSpuThreadAttribute thrattr;
static sysSpuThreadArgument thrargs;

/* Runs every case; returns the failed step, or 0. */
static unsigned int run_cases(u64 e, sys_event_queue_t queue)
{
    unsigned int k, i;
    int ret;

    for (k = 0; k < CASE_COUNT; k++) {
        unsigned int at = 16 + CASE_WORDS * k;
        sys_spu_group_t group;
        sys_spu_thread_t thread;
        sys_event_t event;
        unsigned int cause = 0xFFFFFFFF, status = 0xFFFFFFFF;

        for (i = 0; i < 32; i++)
            landing[k][i] = 0xEEEEEEEEu;
        memset(&event, 0, sizeof(event));
        memset(&grpattr, 0, sizeof(grpattr));
        grpattr.nsize = 9;
        grpattr.name = "test_grp";
        ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
        if (ret != 0) {
            result[1] = (unsigned int)ret;
            return k << 8 | 4;
        }

        memset(&thrattr, 0, sizeof(thrattr));
        thrattr.nsize = 9;
        thrattr.name = "test_spu";
        memset(&thrargs, 0, sizeof(thrargs));
        thrargs.arg1 = e - OFFSET[k];
        thrargs.arg2 = EA(landing[k]);
        ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
        if (ret != 0) {
            result[1] = (unsigned int)ret;
            sysSpuThreadGroupDestroy(group);
            return k << 8 | 5;
        }

        ret = sysSpuThreadGroupConnectEvent(group, queue, EVENT_EXCEPTION);
        if (ret != 0) {
            result[1] = (unsigned int)ret;
            sysSpuThreadGroupDestroy(group);
            return k << 8 | 6;
        }

        ret = sysSpuThreadGroupStart(group);
        if (ret != 0) {
            result[1] = (unsigned int)ret;
            sysSpuThreadGroupDisconnectEvent(group, EVENT_EXCEPTION);
            sysSpuThreadGroupDestroy(group);
            return k << 8 | 7;
        }

        result[at + 0] = (unsigned int)sysEventQueueReceive(queue, &event, RECEIVE_USEC);
        result[at + 1] = (unsigned int)event.data_2;
        if (result[at + 0] == 0)
            result[at + 2] = (unsigned int)sysSpuThreadGroupTerminate(group, TERMINATE_STATUS);
        result[at + 3] = (unsigned int)sysSpuThreadGroupJoin(group, &cause, &status);
        result[at + 4] = cause;
        result[at + 5] = status;
        result[at + 6] = (unsigned int)sysSpuThreadGroupDisconnectEvent(group, EVENT_EXCEPTION);
        result[at + 7] = (unsigned int)sysSpuThreadGroupDestroy(group);
        __sync_synchronize();
        for (i = 0; i < 8; i++)
            result[at + 8 + i] = landing[k][i];
    }
    return 0;
}

int main(void)
{
    unsigned int i, map0 = 0, map1 = 0, m0_ok = 0, m1_ok = 0, failed = 0;
    u64 a = 0, m0 = 0, m1 = 0, e;
    int ret;
    sys_event_queue_t queue;
    sys_event_queue_attr_t qattr;

    for (i = 0; i < WORDS; i++)
        result[i] = (i >= 16) ? SKIP : 0;
    for (i = 4; i < 15; i++)
        result[i] = SKIP;
    result[2] = CASE_COUNT;

    w32 = 0xFFFFFFFFu;
    result[4] = (unsigned int)sc(SYS_MMAPPER_ALLOCATE_ADDRESS, AREA, FLAG_64K, AREA, EA(&w32));
    a = w32;
    result[5] = (unsigned int)a;
    if (result[4] != 0) {
        failed = 1;
        result[1] = result[4];
        goto done;
    }
    w32 = 0xFFFFFFFFu;
    result[6] = (unsigned int)sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, K64, FLAG_64K, EA(&w32));
    m0 = w32;
    m0_ok = result[6] == 0;
    w32 = 0xFFFFFFFFu;
    result[7] = (unsigned int)sc(SYS_MMAPPER_ALLOCATE_SHM, KEYLESS, K64, FLAG_64K, EA(&w32));
    m1 = w32;
    m1_ok = result[7] == 0;
    if (m0_ok && m1_ok) {
        result[8] = (unsigned int)sc(SYS_MMAPPER_MAP_SHM, a, m0, PROT_RW, 0);
        map0 = result[8] == 0;
        if (map0) {
            result[9] = (unsigned int)sc(SYS_MMAPPER_MAP_SHM, a + K64, m1, PROT_RW, 0);
            map1 = result[9] == 0;
        }
    }
    if (!(map0 && map1)) {
        failed = 2;
        goto teardown;
    }

    e = a + K64;
    {
        volatile unsigned int *fill = (volatile unsigned int *)(unsigned long)(e - 48);
        for (i = 0; i < 16; i++) {
            unsigned int b = 0x10 + 4 * i;
            fill[i] = (b << 24) | ((b + 1) << 16) | ((b + 2) << 8) | (b + 3);
        }
    }
    __sync_synchronize();

    ret = CGOV_SPU_IMAGE_OPEN(&image, SPU_ELF_PATH);
    if (ret != 0) {
        failed = 3;
        result[1] = (unsigned int)ret;
        goto teardown;
    }

    memset(&qattr, 0, sizeof(qattr));
    qattr.attr_protocol = SYS_EVENT_QUEUE_FIFO;
    qattr.type = SYS_EVENT_QUEUE_PPU;
    ret = sysEventQueueCreate(&queue, &qattr, SYS_EVENT_QUEUE_KEY_LOCAL, 8);
    if (ret == 0) {
        failed = run_cases(e, queue);
        result[3] = (unsigned int)sysEventQueueDestroy(queue, SYS_EVENT_QUEUE_FORCE_DESTROY);
    } else {
        failed = 8;
        result[1] = (unsigned int)ret;
    }
    sysSpuImageClose(&image);

teardown:
    if (map1)
        result[10] = (unsigned int)unmap(a + K64);
    if (map0)
        result[11] = (unsigned int)unmap(a);
    if (m1_ok)
        result[12] = (unsigned int)sc(SYS_MMAPPER_FREE_SHM, m1, 0, 0, 0);
    if (m0_ok)
        result[13] = (unsigned int)sc(SYS_MMAPPER_FREE_SHM, m0, 0, 0, 0);
    result[14] = (unsigned int)sc(SYS_MMAPPER_FREE_ADDRESS, a, 0, 0, 0);

done:
    result[0] = failed;
    write_frame();
    return 0;
}
