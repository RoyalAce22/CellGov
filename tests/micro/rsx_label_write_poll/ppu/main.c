/* PPU program: brings up an RSX context through libgcm, has the RSX
 * release a value into a label with an NV406E semaphore command,
 * polls the label for it, and exits.
 *
 * Exercises the label-write path the way a game drives it:
 *
 *   1. sys_memory_allocate gives a 1 MiB-page host area, and rsxInit
 *      (libgcm's cellGcmInit) maps it as the RSX IO area and puts the
 *      command buffer at its start. The console's libgcm refuses an IO
 *      area that is not on a 1 MiB boundary, so a static buffer will
 *      not do.
 *   2. rsxSetWriteCommandLabel writes NV406E_SEMAPHORE_OFFSET +
 *      NV406E_SEMAPHORE_RELEASE into the command buffer for user label
 *      LABEL_INDEX; rsxFlushBuffer advances the put pointer.
 *   3. The RSX releases the value into the label, and the spin loop
 *      here sees it.
 *
 * Output layout (TTY payload = CGOV magic + 16 bytes):
 *   status         (u32)  0 = pass; 0x100 = the host allocation
 *                         failed, 0x80 = rsxInit failed (the next word
 *                         then holds its error), 0x40 = no label
 *                         address, 1 = the label never took the value
 *   label_value    (u32)  what the label held at check time
 *                         (expected = magic)
 *   expected       (u32)  the magic the release carried
 *   spin_iters     (u32)  how many poll iterations it took
 */

#include <sys/memory.h>
#include <sys/process.h>
#include <sys/tty.h>
#include <rsx/gcm_sys.h>
#include <rsx/rsx.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

/* The first label libgcm leaves to the program (0-63 are the system's). */
#define LABEL_INDEX 64
#define MAGIC       0xCAFEBABEu
#define HOST_SIZE   (1024 * 1024)
#define CMD_SIZE    0x10000
#define MAX_POLLS   1000000u

#define STATUS_ALLOC_FAILED 0x100u
#define STATUS_INIT_FAILED  0x80u
#define STATUS_NO_LABEL     0x40u
#define STATUS_NOT_RELEASED 0x1u

struct TestResult {
    unsigned int status;
    unsigned int label_value;
    unsigned int expected;
    unsigned int spin_iters;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };
static struct TestResult result;
static gcmContextData *context;

static void write_tty_result(const struct TestResult *r)
{
    unsigned int len = sizeof(*r);
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >> 8) & 0xFF;
    len_be[3] = len & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, r, len, &written);
    CGOV_OUT_FILE_WRITE(r, len);
}

int main(void)
{
    sys_mem_addr_t host = 0;
    volatile u32 *label;
    unsigned int iters;
    s32 rc;

    result.expected = MAGIC;
    if (sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host) != 0) {
        result.status |= STATUS_ALLOC_FAILED;
        write_tty_result(&result);
        return (int)result.status;
    }
    rc = rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    if (rc != 0) {
        result.status |= STATUS_INIT_FAILED;
        result.label_value = (unsigned int)rc;
        write_tty_result(&result);
        return (int)result.status;
    }
    label = gcmGetLabelAddress(LABEL_INDEX);
    if (label == 0) {
        result.status |= STATUS_NO_LABEL;
        write_tty_result(&result);
        return (int)result.status;
    }
    *label = 0;

    rsxSetWriteCommandLabel(context, LABEL_INDEX, MAGIC);
    rsxFlushBuffer(context);

    for (iters = 0; iters < MAX_POLLS; iters++) {
        if (*label == MAGIC)
            break;
    }
    result.spin_iters = iters;
    result.label_value = *label;
    if (result.label_value != MAGIC)
        result.status |= STATUS_NOT_RELEASED;

    write_tty_result(&result);
    return (int)result.status;
}
