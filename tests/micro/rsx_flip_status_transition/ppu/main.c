/* PPU program: brings up an RSX context through libgcm, sets one
 * display buffer, queues a flip to it, polls the flip status through
 * the reset's WAITING to DONE, and exits.
 *
 * Exercises the flip-status state machine the way a game drives it:
 *
 *   1. sys_memory_allocate gives a 1 MiB-page host area, and rsxInit
 *      (libgcm's cellGcmInit) maps it as the RSX IO area.
 *   2. A 1080p colour buffer in RSX local memory becomes display
 *      buffer 0. The video output keeps the mode the XMB left it in.
 *   3. gcmResetFlipStatus sets the status to WAITING; the program reads
 *      it back once.
 *   4. gcmSetFlip queues GCM_FLIP_COMMAND for buffer 0 and
 *      rsxFlushBuffer hands it to the RSX; the program polls
 *      gcmGetFlipStatus until it reads DONE.
 *
 * Output (TTY payload = CGOV magic + 16 bytes):
 *   status          (u32)  0 = pass, else a bitfield:
 *                            0x100 = the host allocation failed,
 *                            0x80 = rsxInit failed (after_reset then
 *                                   holds its error),
 *                            0x08 = no display buffer,
 *                            0x04 = gcmSetFlip refused,
 *                            0x02 = the status never read DONE
 *   after_reset     (u32)  the flip status read right after the reset
 *   done_iters      (u32)  polls from the flush until DONE
 *   last_status     (u32)  the final flip status read
 */

#include <sys/memory.h>
#include <sys/process.h>
#include <sys/tty.h>
#include <rsx/gcm_sys.h>
#include <rsx/rsx.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define HOST_SIZE (1024 * 1024)
#define CMD_SIZE  0x10000
#define MAX_POLLS 50000000u
#define BUFFER_ID 0

#define FLIP_STATUS_DONE 0u

/* The display buffer covers the largest output mode, 1080p, so it
 * fits whatever mode the XMB left the video output in; the program
 * leaves that mode alone. */
#define DISPLAY_WIDTH  1920u
#define DISPLAY_HEIGHT 1080u

#define STATUS_ALLOC_FAILED     0x100u
#define STATUS_INIT_FAILED      0x80u
#define STATUS_NO_DISPLAY       0x08u
#define STATUS_FLIP_REFUSED     0x04u
#define STATUS_NEVER_DONE       0x02u

struct TestResult {
    unsigned int status;
    unsigned int after_reset;
    unsigned int done_iters;
    unsigned int last_status;
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

static int finish(void)
{
    write_tty_result(&result);
    return (int)result.status;
}

int main(void)
{
    sys_mem_addr_t host = 0;
    void *buffer;
    u32 offset = 0;
    u32 pitch;
    unsigned int iters;
    s32 rc;

    if (sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host) != 0) {
        result.status |= STATUS_ALLOC_FAILED;
        return finish();
    }
    rc = rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    if (rc != 0) {
        result.status |= STATUS_INIT_FAILED;
        result.after_reset = (unsigned int)rc;
        return finish();
    }

    pitch = DISPLAY_WIDTH * 4;
    buffer = rsxMemalign(64, pitch * DISPLAY_HEIGHT);
    if (buffer == 0 || gcmAddressToOffset(buffer, &offset) != 0
        || gcmSetDisplayBuffer(BUFFER_ID, offset, pitch, DISPLAY_WIDTH,
                               DISPLAY_HEIGHT) != 0) {
        result.status |= STATUS_NO_DISPLAY;
        return finish();
    }

    gcmSetFlipMode(GCM_FLIP_VSYNC);
    gcmResetFlipStatus();
    result.after_reset = gcmGetFlipStatus();

    if (gcmSetFlip(context, BUFFER_ID) != 0) {
        result.status |= STATUS_FLIP_REFUSED;
        return finish();
    }
    rsxFlushBuffer(context);

    for (iters = 0; iters < MAX_POLLS; iters++) {
        if (gcmGetFlipStatus() == FLIP_STATUS_DONE)
            break;
    }
    result.done_iters = iters;
    result.last_status = gcmGetFlipStatus();
    if (result.last_status != FLIP_STATUS_DONE)
        result.status |= STATUS_NEVER_DONE;
    return finish();
}
