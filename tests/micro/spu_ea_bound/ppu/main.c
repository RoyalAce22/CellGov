/* PPU program: run the spu_ea_bound SPU program once per case, each in a
 * fresh one-thread group whose exception event goes to one PPU event
 * queue, and record what the kernel reports.
 *
 * Cases (the access, then the effective address):
 *   0..3  a 16-byte get, EAH 0x1, 0xFF, 0x3FF, 0x400 with EAL 0x10000
 *   4..7  a getllar,     EAH 0x1, 0xFF, 0x3FF, 0x400 with EAL 0x10000
 *   8..9  a 16-byte get, 0x20000000 and 0x0F000000, which the process
 *         maps nothing at
 *
 * Per case: create the group and its thread, connect the queue to the
 * group's exception event, start the group, receive one event (5 s
 * timeout), terminate the group, join it, disconnect the event and
 * destroy the group.
 *
 * Result frame (16 + 48 * CASE_COUNT bytes, big-endian):
 *   +0   u32 failed step (case << 8 | step, 0 = none), u32 its return
 *        code, u32 CASE_COUNT, u32 the queue destroy's answer
 *   +16 + 48k, case k:
 *        u32 receive's answer, u32 the event source's low word,
 *        u32 data2 high (SPU_NPC), u32 data2 low (cause),
 *        u32 data3 high, u32 data3 low,
 *        u32 terminate's answer, u32 join's answer, u32 join cause,
 *        u32 join status, u32 disconnect's answer, u32 destroy's answer
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <sys/event_queue.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define CASE_COUNT 10
#define RESULT_BYTES (16 + 48 * CASE_COUNT)
#define EVENT_EXCEPTION 2
#define RECEIVE_USEC 5000000ull
#define TERMINATE_STATUS 0x7E57

static const unsigned long long EA[CASE_COUNT] = {
    0x0000000100010000ull, 0x000000FF00010000ull,
    0x000003FF00010000ull, 0x0000040000010000ull,
    0x0000000100010000ull, 0x000000FF00010000ull,
    0x000003FF00010000ull, 0x0000040000010000ull,
    0x0000000020000000ull, 0x000000000F000000ull,
};
/* 0 a get, 1 a getllar. */
static const unsigned int ACCESS[CASE_COUNT] = { 0, 0, 0, 0, 1, 1, 1, 1, 0, 0 };

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static unsigned char results[RESULT_BYTES] __attribute__((aligned(128)));

static void write_frame(const void *payload, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, payload, len, &written);
    CGOV_OUT_FILE_WRITE(payload, len);
}

static void put_u32(unsigned int at, unsigned int value)
{
    int i;
    for (i = 0; i < 4; i++)
        results[at + i] = (unsigned char)(value >> (24 - 8 * i));
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    put_u32(0, step);
    put_u32(4, (unsigned int)rc);
    write_frame(results, RESULT_BYTES);
    return (int)step;
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

static sysSpuImage image;
static sysSpuThreadGroupAttribute grpattr;
static sysSpuThreadAttribute thrattr;
static sysSpuThreadArgument thrargs;

int main(void)
{
    int ret;
    unsigned int k;
    sys_event_queue_t queue;
    sys_event_queue_attr_t qattr;

    memset(results, 0, sizeof(results));
    put_u32(8, CASE_COUNT);

    ret = CGOV_SPU_IMAGE_OPEN(&image, SPU_ELF_PATH);
    if (ret != 0)
        return fail(1, ret);

    memset(&qattr, 0, sizeof(qattr));
    qattr.attr_protocol = SYS_EVENT_QUEUE_FIFO;
    qattr.type = SYS_EVENT_QUEUE_PPU;
    ret = sysEventQueueCreate(&queue, &qattr, SYS_EVENT_QUEUE_KEY_LOCAL, 8);
    if (ret != 0)
        return fail(2, ret);

    for (k = 0; k < CASE_COUNT; k++) {
        unsigned int at = 16 + 48 * k;
        sys_spu_group_t group;
        sys_spu_thread_t thread;
        sys_event_t event;
        unsigned int cause = 0xFFFFFFFF, status = 0xFFFFFFFF;

        memset(&event, 0, sizeof(event));
        memset(&grpattr, 0, sizeof(grpattr));
        grpattr.nsize = 9;
        grpattr.name = "test_grp";
        ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
        if (ret != 0)
            return fail(k << 8 | 3, ret);

        memset(&thrattr, 0, sizeof(thrattr));
        thrattr.nsize = 9;
        thrattr.name = "test_spu";
        memset(&thrargs, 0, sizeof(thrargs));
        thrargs.arg1 = EA[k];
        thrargs.arg2 = ACCESS[k];
        ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
        if (ret != 0)
            return fail(k << 8 | 4, ret);

        ret = sysSpuThreadGroupConnectEvent(group, queue, EVENT_EXCEPTION);
        if (ret != 0)
            return fail(k << 8 | 5, ret);

        ret = sysSpuThreadGroupStart(group);
        if (ret != 0)
            return fail(k << 8 | 6, ret);

        put_u32(at, (unsigned int)sysEventQueueReceive(queue, &event, RECEIVE_USEC));
        put_u32(at + 4, (unsigned int)event.source);
        put_u32(at + 8, (unsigned int)(event.data_2 >> 32));
        put_u32(at + 12, (unsigned int)event.data_2);
        put_u32(at + 16, (unsigned int)(event.data_3 >> 32));
        put_u32(at + 20, (unsigned int)event.data_3);

        put_u32(at + 24, (unsigned int)sysSpuThreadGroupTerminate(group, TERMINATE_STATUS));
        put_u32(at + 28, (unsigned int)sysSpuThreadGroupJoin(group, &cause, &status));
        put_u32(at + 32, cause);
        put_u32(at + 36, status);
        put_u32(at + 40, (unsigned int)sysSpuThreadGroupDisconnectEvent(group, EVENT_EXCEPTION));
        put_u32(at + 44, (unsigned int)sysSpuThreadGroupDestroy(group));
    }

    put_u32(12, (unsigned int)sysEventQueueDestroy(queue, SYS_EVENT_QUEUE_FORCE_DESTROY));
    write_frame(results, RESULT_BYTES);
    sysSpuImageClose(&image);
    return 0;
}
