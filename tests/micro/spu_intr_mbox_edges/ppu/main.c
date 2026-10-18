/* PPU program: two edges of the user events an SPU thread sends
 * through its outbound interrupt mailbox.
 *
 * One group runs one thread of spu/main.c. Before the start the PPU
 * connects queue 1 to the thread's port 3 and queue 2 to its port 5.
 * After the start it puts OCCUPY_WORD in the thread's inbound mailbox
 * and writes signal register 1, so the SPU sends on port 5 while that
 * word waits. It then destroys queue 1, without and then with the
 * force flag if the first is refused, and writes signal register 2, so
 * the SPU sends and throws on port 3. After the join it receives from
 * queue 2 until it is empty.
 *
 * Result layout (80 bytes, big-endian):
 *   +0:  u32 failed step (0 = none), u32 its return code,
 *        u32 queue 1 destroy answer, u32 forced destroy answer (0 when
 *        the first destroy took it)
 *   +16: the SPU's words: u32 SPU_RdInMbox count before the send,
 *        u32 first inbound read, u32 count after the spin, u32 second
 *        inbound read (0xEEEEEEEE when nothing came), u32 send answer
 *        on the destroyed queue's port, u32 SPU_RdInMbox count after
 *        the throw, u32 SPU_WrOutIntrMbox count, u32 0
 *   +48: u32 queue 2 first receive answer, u32 its data1 is the thread
 *        id, u32 its data2 high, u32 its data2 low, u32 its data3 low,
 *        u32 queue 2 second receive answer, u32 0, u32 0
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

#define RESULT_BYTES 80

#define DESTROYED_PORT  3
#define OCCUPIED_PORT   5
#define OCCUPY_WORD     0x5A5A5A5Au
#define RECEIVE_TIMEOUT 100000

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
static int create_queue(sys_event_queue_t *queue)
{
    volatile sys_event_queue_attr_t qattr;

    memset((void *)&qattr, 0, sizeof(qattr));
    qattr.attr_protocol = SYS_EVENT_QUEUE_FIFO;
    qattr.type = SYS_EVENT_QUEUE_PPU;
    return sysEventQueueCreate(queue, (sys_event_queue_attr_t *)&qattr,
                               SYS_EVENT_QUEUE_KEY_LOCAL, 4);
}

int main(void)
{
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    sys_event_queue_t queue1, queue2;
    unsigned int cause, status;
    int queue1_live = 1;
    int ret;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;
    result_buf[3] = 0;

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1, ret);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "intredg";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &g_grpattr);
    if (ret != 0) return fail(2, ret);
    ret = create_queue(&queue1);
    if (ret != 0) return fail(3, ret);
    ret = create_queue(&queue2);
    if (ret != 0) return fail(4, ret);

    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 8;
    g_thrattr.name = "intredg";
    memset(&g_args, 0, sizeof(g_args));
    g_args.arg1 = (u64)(unsigned long)result_buf;
    ret = sysSpuThreadInitialize(&thread, group, 0, &g_image, &g_thrattr, &g_args);
    if (ret != 0) return fail(5, ret);
    ret = sysSpuThreadConnectEvent(thread, queue1, SPU_THREAD_EVENT_USER, DESTROYED_PORT);
    if (ret != 0) return fail(6, ret);
    ret = sysSpuThreadConnectEvent(thread, queue2, SPU_THREAD_EVENT_USER, OCCUPIED_PORT);
    if (ret != 0) return fail(7, ret);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(8, ret);
    ret = sysSpuThreadWriteMb(thread, OCCUPY_WORD);
    if (ret != 0) return fail(9, ret);
    ret = sysSpuThreadWriteSignal(thread, 0, 1);
    if (ret != 0) return fail(10, ret);

    result_buf[2] = (unsigned int)sysEventQueueDestroy(queue1, 0);
    if (result_buf[2] != 0)
        result_buf[3] = (unsigned int)sysEventQueueDestroy(queue1, SYS_EVENT_QUEUE_FORCE_DESTROY);
    if (result_buf[2] == 0 || result_buf[3] == 0)
        queue1_live = 0;
    ret = sysSpuThreadWriteSignal(thread, 1, 1);
    if (ret != 0) return fail(11, ret);

    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(12, ret);

    memset(&g_event, 0, sizeof(g_event));
    result_buf[12] = (unsigned int)sysEventQueueReceive(queue2, &g_event, RECEIVE_TIMEOUT);
    result_buf[13] = g_event.data_1 == (u64)thread;
    result_buf[14] = (unsigned int)(g_event.data_2 >> 32);
    result_buf[15] = (unsigned int)g_event.data_2;
    result_buf[16] = (unsigned int)g_event.data_3;
    memset(&g_event, 0, sizeof(g_event));
    result_buf[17] = (unsigned int)sysEventQueueReceive(queue2, &g_event, RECEIVE_TIMEOUT);
    result_buf[18] = 0;
    result_buf[19] = 0;

    sysSpuThreadGroupDestroy(group);
    if (queue1_live)
        sysEventQueueDestroy(queue1, SYS_EVENT_QUEUE_FORCE_DESTROY);
    sysEventQueueDestroy(queue2, SYS_EVENT_QUEUE_FORCE_DESTROY);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
