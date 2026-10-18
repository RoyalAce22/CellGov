/* PPU program: the events an SPU thread's outbound interrupt mailbox
 * sends to the event queues connected to its ports.
 *
 * One group runs one thread of spu/main.c. Before the start the PPU
 * connects queue 1 (2 entries) to the thread's port 3 with
 * sys_spu_thread_connect_event, and queue 2 (4 entries) to one port of
 * every thread with sys_spu_thread_group_connect_event_all_threads,
 * asking for port 10. After the join it receives from both queues
 * until each is empty.
 *
 * Result layout (144 bytes, big-endian):
 *   +0:   u32 failed step (0 = none), u32 its return code,
 *         u32 the group port, u32 0
 *   +16:  the SPU's words: u32 SPU_WrOutIntrMbox count before, u32 send
 *         answer, u32 send answer on the full queue, u32 send answer on
 *         the unconnected port, u32 send answer on the group port,
 *         u32 SPU_RdInMbox count, u32 SPU_WrOutIntrMbox count, u32 0
 *   +48:  queue 1, first event: u32 receive answer, u32 source high,
 *         u32 source low, u32 data1 is the thread id (1 or 0),
 *         u32 data2 high, u32 data2 low, u32 data3 high, u32 data3 low
 *   +80:  queue 1, second event, the same 8 words
 *   +112: u32 queue 1 third receive answer, u32 queue 2 first receive
 *         answer, u32 its data1 is the thread id, u32 its data2 high,
 *         u32 its data2 low, u32 its data3 low, u32 queue 2 second
 *         receive answer, u32 0
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

#define RESULT_BYTES 144

#define CONNECTED_PORT   3
#define GROUP_PORT_ASKED 10
#define RECEIVE_TIMEOUT  100000

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile unsigned int result_buf[RESULT_BYTES / 4] __attribute__((aligned(128)));

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

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

static sysSpuImage g_image;
static sysSpuThreadGroupAttribute g_grpattr;
static sysSpuThreadAttribute g_thrattr;
static sysSpuThreadArgument g_args;
static sys_event_t g_event;

/* The attribute is volatile so the two fields are stored one word at a
 * time. Merged, they form a 64-bit constant whose high word the
 * emulator build's rldicr rewrite (common/patch_toc.py) clears. */
static int create_queue(sys_event_queue_t *queue, int size)
{
    volatile sys_event_queue_attr_t qattr;

    memset((void *)&qattr, 0, sizeof(qattr));
    qattr.attr_protocol = SYS_EVENT_QUEUE_FIFO;
    qattr.type = SYS_EVENT_QUEUE_PPU;
    return sysEventQueueCreate(queue, (sys_event_queue_attr_t *)&qattr,
                               SYS_EVENT_QUEUE_KEY_LOCAL, size);
}

/* Receive one event and write its 8 words at result_buf[at]. */
static void receive_full(sys_event_queue_t queue, sys_spu_thread_t thread, unsigned int at)
{
    memset(&g_event, 0, sizeof(g_event));
    result_buf[at] = (unsigned int)sysEventQueueReceive(queue, &g_event, RECEIVE_TIMEOUT);
    result_buf[at + 1] = (unsigned int)(g_event.source >> 32);
    result_buf[at + 2] = (unsigned int)g_event.source;
    result_buf[at + 3] = g_event.data_1 == (u64)thread;
    result_buf[at + 4] = (unsigned int)(g_event.data_2 >> 32);
    result_buf[at + 5] = (unsigned int)g_event.data_2;
    result_buf[at + 6] = (unsigned int)(g_event.data_3 >> 32);
    result_buf[at + 7] = (unsigned int)g_event.data_3;
}

int main(void)
{
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    sys_event_queue_t queue1, queue2;
    u8 group_port = 0xFF;
    unsigned int cause, status;
    int ret;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;
    result_buf[3] = 0;

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1, ret);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "intrmbx";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &g_grpattr);
    if (ret != 0) return fail(2, ret);

    ret = create_queue(&queue1, 2);
    if (ret != 0) return fail(3, ret);
    ret = create_queue(&queue2, 4);
    if (ret != 0) return fail(4, ret);

    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 8;
    g_thrattr.name = "intrmbx";
    memset(&g_args, 0, sizeof(g_args));
    g_args.arg1 = (u64)(unsigned long)result_buf;
    /* The mask asks for one port, so the connect below can only give
     * that one; the SPU takes it from arg2 and the PPU records the
     * port the kernel wrote. */
    g_args.arg2 = GROUP_PORT_ASKED;
    ret = sysSpuThreadInitialize(&thread, group, 0, &g_image, &g_thrattr, &g_args);
    if (ret != 0) return fail(5, ret);

    ret = sysSpuThreadConnectEvent(thread, queue1, SPU_THREAD_EVENT_USER, CONNECTED_PORT);
    if (ret != 0) return fail(6, ret);
    ret = sysSpuThreadGroupConnectEventAllThreads(group, queue2,
                                                  1ULL << GROUP_PORT_ASKED, &group_port);
    if (ret != 0) return fail(7, ret);
    result_buf[2] = group_port;

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(8, ret);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(9, ret);

    receive_full(queue1, thread, 12);
    receive_full(queue1, thread, 20);
    memset(&g_event, 0, sizeof(g_event));
    result_buf[28] = (unsigned int)sysEventQueueReceive(queue1, &g_event, RECEIVE_TIMEOUT);

    memset(&g_event, 0, sizeof(g_event));
    result_buf[29] = (unsigned int)sysEventQueueReceive(queue2, &g_event, RECEIVE_TIMEOUT);
    result_buf[30] = g_event.data_1 == (u64)thread;
    result_buf[31] = (unsigned int)(g_event.data_2 >> 32);
    result_buf[32] = (unsigned int)g_event.data_2;
    result_buf[33] = (unsigned int)g_event.data_3;
    memset(&g_event, 0, sizeof(g_event));
    result_buf[34] = (unsigned int)sysEventQueueReceive(queue2, &g_event, RECEIVE_TIMEOUT);
    result_buf[35] = 0;

    sysSpuThreadGroupDestroy(group);
    sysEventQueueDestroy(queue1, SYS_EVENT_QUEUE_FORCE_DESTROY);
    sysEventQueueDestroy(queue2, SYS_EVENT_QUEUE_FORCE_DESTROY);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
