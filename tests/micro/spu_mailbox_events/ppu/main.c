/* PPU program: host the spu_mailbox_events SPU program.
 *
 * One group runs one thread of spu/main.c. Before the start the PPU
 * connects a queue to the thread's port 1, so LV2 reads both outbound
 * mailboxes when the thread writes SPU_WrOutIntrMbox. After the join it
 * receives the send's event and the throw's.
 *
 * Result layout (96 bytes, big-endian):
 *   +0:  u32 failed step (0 = none), u32 its return code, u32 first
 *        receive answer, u32 second receive answer
 *   +16: the SPU's 16 words (spu/main.c lists them)
 *   +80: u32 first event's data3 low, u32 second event's data3 low,
 *        u32 second event's data2 low, u32 0
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

#define RESULT_BYTES 96

#define PORT            1
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
    sys_event_queue_t queue;
    unsigned int cause, status;
    int ret;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1, ret);

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "mbxevnt";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &g_grpattr);
    if (ret != 0) return fail(2, ret);
    ret = create_queue(&queue);
    if (ret != 0) return fail(3, ret);

    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 8;
    g_thrattr.name = "mbxevnt";
    memset(&g_args, 0, sizeof(g_args));
    g_args.arg1 = (u64)(unsigned long)result_buf;
    ret = sysSpuThreadInitialize(&thread, group, 0, &g_image, &g_thrattr, &g_args);
    if (ret != 0) return fail(4, ret);
    ret = sysSpuThreadConnectEvent(thread, queue, SPU_THREAD_EVENT_USER, PORT);
    if (ret != 0) return fail(5, ret);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(6, ret);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(7, ret);

    memset(&g_event, 0, sizeof(g_event));
    result_buf[2] = (unsigned int)sysEventQueueReceive(queue, &g_event, RECEIVE_TIMEOUT);
    result_buf[20] = (unsigned int)g_event.data_3;
    memset(&g_event, 0, sizeof(g_event));
    result_buf[3] = (unsigned int)sysEventQueueReceive(queue, &g_event, RECEIVE_TIMEOUT);
    result_buf[21] = (unsigned int)g_event.data_3;
    result_buf[22] = (unsigned int)g_event.data_2;
    result_buf[23] = 0;

    sysSpuThreadGroupDestroy(group);
    sysEventQueueDestroy(queue, SYS_EVENT_QUEUE_FORCE_DESTROY);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
