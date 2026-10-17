/* PPU program: host the spu_list_bad_ea SPU program in a one-thread
 * group whose exception event goes to a PPU event queue, and record what
 * the kernel reports and which list elements reached the buffer.
 *
 * Steps: open the image, create the event queue, create the group and
 * its thread, connect the queue to the group's exception event, start
 * the group, receive one event (5 s timeout), terminate the group, join
 * it, disconnect the event, destroy the group and the queue.
 *
 * Result frame (112 bytes, big-endian):
 *   +0   u32 failed step (0 = none), u32 its return code
 *   +8   u32 the receive's answer, u32 0
 *   +16  u64 event source, u64 data1, u64 data2, u64 data3
 *   +48  u32 terminate's answer, u32 join's answer, u32 join cause,
 *        u32 join status
 *   +64  u32 disconnect's answer, u32 group destroy's answer,
 *        u32 queue destroy's answer, u32 0
 *   +80  u32 group id, u32 thread id, u32 buffer EA, u32 0
 *   +96  u32 the buffer's first word at +0x100, +0x180, +0x184, +0x200
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

#define RESULT_BYTES 112
#define EVENT_EXCEPTION 2
#define RECEIVE_USEC 5000000ull
#define TERMINATE_STATUS 0x7E57

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static unsigned char results[RESULT_BYTES] __attribute__((aligned(128)));
/* Room for the bad element in full, should the MFC move it. */
static unsigned char buffer[0x5000] __attribute__((aligned(128)));

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

static void put_u64(unsigned int at, unsigned long long value)
{
    put_u32(at, (unsigned int)(value >> 32));
    put_u32(at + 4, (unsigned int)value);
}

static unsigned int buffer_word(unsigned int offset)
{
    const unsigned char *p = buffer + offset;
    return ((unsigned int)p[0] << 24) | ((unsigned int)p[1] << 16)
         | ((unsigned int)p[2] << 8) | p[3];
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    put_u32(0, step);
    put_u32(4, (unsigned int)rc);
    write_frame(results, RESULT_BYTES);
    return (int)step;
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

int main(void)
{
    int ret;
    sysSpuImage image;
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    sysSpuThreadGroupAttribute grpattr;
    sysSpuThreadAttribute thrattr;
    sysSpuThreadArgument thrargs;
    sys_event_queue_t queue;
    sys_event_queue_attr_t qattr;
    sys_event_t event;
    unsigned int cause = 0xFFFFFFFF, status = 0xFFFFFFFF;

    memset(results, 0, sizeof(results));
    memset(buffer, 0, sizeof(buffer));
    memset(&event, 0, sizeof(event));

    ret = CGOV_SPU_IMAGE_OPEN(&image, SPU_ELF_PATH);
    if (ret != 0)
        return fail(1, ret);

    memset(&qattr, 0, sizeof(qattr));
    qattr.attr_protocol = SYS_EVENT_QUEUE_FIFO;
    qattr.type = SYS_EVENT_QUEUE_PPU;
    ret = sysEventQueueCreate(&queue, &qattr, SYS_EVENT_QUEUE_KEY_LOCAL, 8);
    if (ret != 0)
        return fail(2, ret);

    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 9;
    grpattr.name = "test_grp";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
    if (ret != 0)
        return fail(3, ret);

    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 9;
    thrattr.name = "test_spu";
    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = (u64)(unsigned long)buffer;
    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0)
        return fail(4, ret);

    ret = sysSpuThreadGroupConnectEvent(group, queue, EVENT_EXCEPTION);
    if (ret != 0)
        return fail(5, ret);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0)
        return fail(6, ret);

    put_u32(8, (unsigned int)sysEventQueueReceive(queue, &event, RECEIVE_USEC));
    put_u64(16, event.source);
    put_u64(24, event.data_1);
    put_u64(32, event.data_2);
    put_u64(40, event.data_3);

    put_u32(48, (unsigned int)sysSpuThreadGroupTerminate(group, TERMINATE_STATUS));
    put_u32(52, (unsigned int)sysSpuThreadGroupJoin(group, &cause, &status));
    put_u32(56, cause);
    put_u32(60, status);

    put_u32(64, (unsigned int)sysSpuThreadGroupDisconnectEvent(group, EVENT_EXCEPTION));
    put_u32(68, (unsigned int)sysSpuThreadGroupDestroy(group));
    put_u32(72, (unsigned int)sysEventQueueDestroy(queue, SYS_EVENT_QUEUE_FORCE_DESTROY));

    put_u32(80, (unsigned int)group);
    put_u32(84, (unsigned int)thread);
    put_u32(88, (unsigned int)(unsigned long)buffer);

    put_u32(96, buffer_word(0x100));
    put_u32(100, buffer_word(0x180));
    put_u32(104, buffer_word(0x184));
    put_u32(108, buffer_word(0x200));

    write_frame(results, RESULT_BYTES);
    sysSpuImageClose(&image);
    return 0;
}
