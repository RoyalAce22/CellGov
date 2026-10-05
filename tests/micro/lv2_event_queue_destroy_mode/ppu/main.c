/* PPU probe: what sys_event_queue_destroy (129) answers for a mode
 * word other than 0 and FORCE (1), and which it checks first, the id
 * or the mode.
 *
 * Steps, each recording its return code:
 *   1. sc 128 creates an empty local PPU queue.
 *   2. sc 129(id, 2)
 *   3. sc 129(id, 0x80000000)
 *   4. sc 129(0xDEADBEEF, 2)   (stale id, unlisted mode)
 *   5. sc 129(id, 0), which destroys the queue if 2 and 3 left it.
 *
 * Payload layout (6 big-endian u32 words, 24 bytes):
 *   +0  u32 1 when the create failed, else 0
 *   +4  u32 the create's return code
 *   +8  u32 step 2
 *   +12 u32 step 3
 *   +16 u32 step 4
 *   +20 u32 step 5
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_EVENT_QUEUE_CREATE   128
#define SYS_EVENT_QUEUE_DESTROY  129

/* sys_event_queue_attribute_t: protocol@0 (FIFO), type@4 (PPU queue),
 * name@8. */
static const unsigned int queue_attr[4] __attribute__((aligned(8))) = { 1, 1, 0, 0 };

static unsigned int result[6] __attribute__((aligned(128)));
static unsigned int queue_id __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_result(void)
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
    sysTtyWrite(0, result, len, &written);
    CGOV_OUT_FILE_WRITE(result, len);
}

#define EA(p) ((unsigned long)(const void *)(p))

int main(void)
{
    s32 rc;

    memset(result, 0, sizeof(result));
    rc = syscall4_s32(SYS_EVENT_QUEUE_CREATE, EA(&queue_id), EA(queue_attr), 0, 16);
    if (rc != 0) {
        result[0] = 1;
        result[1] = (unsigned int)rc;
        write_result();
        return 1;
    }
    result[2] = (unsigned int)syscall2_s32(SYS_EVENT_QUEUE_DESTROY, queue_id, 2);
    result[3] = (unsigned int)syscall2_s32(SYS_EVENT_QUEUE_DESTROY, queue_id, 0x80000000u);
    result[4] = (unsigned int)syscall2_s32(SYS_EVENT_QUEUE_DESTROY, 0xDEADBEEFu, 2);
    result[5] = (unsigned int)syscall2_s32(SYS_EVENT_QUEUE_DESTROY, queue_id, 0);
    write_result();
    return 0;
}
