/* PPU program: the join cause and status of a terminated SPU thread
 * group, and what the event-queue stop-and-signal services answer.
 *
 * Group 1 runs one thread of spu/main.c in MODE_PARK. The PPU starts
 * it, terminates the group with TERMINATE_STATUS and joins it.
 *
 * Group 2 runs one thread in MODE_EVENTS. Before the start the PPU
 * creates an SPU event queue, sends one event (EVENT_DATA1-3) into it
 * through a local port, and binds the queue to the thread at spuq 5.
 * The SPU then receives from spuq 5, tries again on the empty queue,
 * and tries spuq 6, which nothing is bound to.
 *
 * Result layout (64 bytes, big-endian):
 *   +0:  u32 failed step (0 = none), u32 its return code,
 *        u32 sys_spu_thread_group_terminate's answer, u32 0
 *   +16: u32 group 1 join cause, u32 group 1 join status,
 *        u32 group 2 join cause, u32 group 2 join status
 *   +32: u32 receive answer, u32 the three words it received,
 *   +48: u32 tryreceive on the empty queue, u32 tryreceive on spuq 6,
 *        u32 0, u32 0
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

#define RESULT_BYTES 64

#define MODE_PARK   1
#define MODE_EVENTS 2

#define BOUND_SPUQ       5
#define TERMINATE_STATUS 0x7E57
#define EVENT_DATA1      0x11111111u
#define EVENT_DATA2      0x22222222u
#define EVENT_DATA3      0x33333333u

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
static sys_event_queue_attr_t g_qattr;

/* Create a one-thread group running spu/main.c in `mode`. */
static int create_group(sys_spu_group_t *group, unsigned int mode,
                        sys_spu_thread_t *thread)
{
    int ret;

    memset(&g_grpattr, 0, sizeof(g_grpattr));
    g_grpattr.nsize = 8;
    g_grpattr.name = "abistop";
    ret = sysSpuThreadGroupCreate(group, 1, 100, &g_grpattr);
    if (ret != 0)
        return ret;

    memset(&g_thrattr, 0, sizeof(g_thrattr));
    g_thrattr.nsize = 8;
    g_thrattr.name = "abistop";
    memset(&g_args, 0, sizeof(g_args));
    g_args.arg0 = mode;
    g_args.arg1 = (u64)(unsigned long)result_buf;
    return sysSpuThreadInitialize(thread, *group, 0, &g_image, &g_thrattr, &g_args);
}

int main(void)
{
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    sys_event_queue_t queue;
    sys_event_port_t port;
    unsigned int cause, status;
    int ret;

    memset((void *)result_buf, 0xEE, sizeof(result_buf));
    result_buf[0] = 0;
    result_buf[1] = 0;
    result_buf[3] = 0;

    ret = CGOV_SPU_IMAGE_OPEN(&g_image, SPU_ELF_PATH);
    if (ret != 0) return fail(1, ret);

    ret = create_group(&group, MODE_PARK, &thread);
    if (ret != 0) return fail(2, ret);
    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(3, ret);
    result_buf[2] = (unsigned int)sysSpuThreadGroupTerminate(group, TERMINATE_STATUS);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(4, ret);
    result_buf[4] = cause;
    result_buf[5] = status;
    ret = sysSpuThreadGroupDestroy(group);
    if (ret != 0) return fail(5, ret);

    memset(&g_qattr, 0, sizeof(g_qattr));
    g_qattr.attr_protocol = SYS_EVENT_QUEUE_FIFO;
    g_qattr.type = SYS_EVENT_QUEUE_SPU;
    ret = sysEventQueueCreate(&queue, &g_qattr, SYS_EVENT_QUEUE_KEY_LOCAL, 4);
    if (ret != 0) return fail(6, ret);
    ret = sysEventPortCreate(&port, SYS_EVENT_PORT_LOCAL, SYS_EVENT_PORT_NO_NAME);
    if (ret != 0) return fail(7, ret);
    ret = sysEventPortConnectLocal(port, queue);
    if (ret != 0) return fail(8, ret);
    ret = sysEventPortSend(port, EVENT_DATA1, EVENT_DATA2, EVENT_DATA3);
    if (ret != 0) return fail(9, ret);

    ret = create_group(&group, MODE_EVENTS, &thread);
    if (ret != 0) return fail(10, ret);
    ret = sysSpuThreadBindQueue(thread, queue, BOUND_SPUQ);
    if (ret != 0) return fail(11, ret);
    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(12, ret);
    cause = status = 0xFFFFFFFF;
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(13, ret);
    result_buf[6] = cause;
    result_buf[7] = status;

    sysSpuThreadGroupDestroy(group);
    sysEventPortDisconnect(port);
    sysEventPortDestroy(port);
    sysEventQueueDestroy(queue, SYS_EVENT_QUEUE_FORCE_DESTROY);

    write_frame(result_buf, RESULT_BYTES);
    return 0;
}
