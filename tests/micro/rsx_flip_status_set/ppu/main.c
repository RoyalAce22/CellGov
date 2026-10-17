/* PPU program: brings up an RSX context through libgcm, then drives
 * the flip status by hand -- reset, set, reset, set -- reading it back
 * after each call. No flip is queued, so only the reset and set calls
 * move the status.
 *
 * Output (TTY payload = CGOV magic + 20 bytes, big-endian u32):
 *   init_rc        rsxInit's return code (0 = the context came up;
 *                  the four reads are 0xFFFFFFFF otherwise)
 *   after_reset    gcmGetFlipStatus after gcmResetFlipStatus
 *   after_set      gcmGetFlipStatus after gcmSetFlipStatus
 *   after_reset2   gcmGetFlipStatus after a second gcmResetFlipStatus
 *   after_set2     gcmGetFlipStatus after a second gcmSetFlipStatus
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

struct TestResult {
    unsigned int init_rc;
    unsigned int after_reset;
    unsigned int after_set;
    unsigned int after_reset2;
    unsigned int after_set2;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };
static struct TestResult result = {
    0xFFFFFFFFu, 0xFFFFFFFFu, 0xFFFFFFFFu, 0xFFFFFFFFu, 0xFFFFFFFFu
};
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
    s32 rc;

    rc = sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host);
    if (rc == 0)
        rc = rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    result.init_rc = (unsigned int)rc;
    if (rc == 0) {
        gcmResetFlipStatus();
        result.after_reset = gcmGetFlipStatus();
        gcmSetFlipStatus();
        result.after_set = gcmGetFlipStatus();
        gcmResetFlipStatus();
        result.after_reset2 = gcmGetFlipStatus();
        gcmSetFlipStatus();
        result.after_set2 = gcmGetFlipStatus();
    }
    write_tty_result(&result);
    return 0;
}
