/* PPU program: brings up an RSX context through libgcm, asks the RSX
 * for a report with NV4097_GET_REPORT, polls the report record for a
 * timestamp, and exits.
 *
 * Exercises the SetReport path. libgcm's cellGcmSetReport emits this
 * method; the RSX writes a report record { u64 timer; u32 value;
 * u32 zero } at the report index the argument names, in the report
 * area libgcm's init sets up, which gcmGetReportDataAddress returns.
 *
 * Test name / TTY verdict string: this microtest lives in
 * `rsx_semaphore_post/` because Sony's libgcm groups SetReport alongside
 * the semaphore-release family under the "back-end semaphore post"
 * label (gcm_implementation_sub.h). The method exercised is
 * GET_REPORT / 0x1800 specifically.
 *
 * Command buffer:
 *   [0] NV4097_GET_REPORT header (method 0x1800, count 1)
 *   [1] report argument: the report type in the top 8 bits
 *       (ZPASS_PIXEL_CNT), the record's byte offset (index * 16) below
 *
 * Output (TTY payload = CGOV magic + 16 bytes):
 *   status         (u32)  0 = pass; 0x100 = the host allocation
 *                         failed, 0x80 = rsxInit failed (the next word
 *                         then holds its error), 0x40 = no report
 *                         address, 1 = the timer stayed 0
 *   timer_low      (u32)  low 32 bits of the record's timer
 *   spin_iters     (u32)  iterations until the timer became non-zero
 *   value          (u32)  the record's value: the Z-pass pixel count,
 *                         0 with nothing drawn
 */

#include <sys/memory.h>
#include <sys/process.h>
#include <sys/tty.h>
#include <rsx/gcm_sys.h>
#include <rsx/rsx.h>

#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define NV4097_GET_REPORT 0x1800
#define NV_COUNT_SHIFT    18
#define REPORT_INDEX      1
#define HOST_SIZE         (1024 * 1024)
#define CMD_SIZE          0x10000
#define MAX_POLLS         1000000u

#define STATUS_ALLOC_FAILED 0x100u
#define STATUS_INIT_FAILED  0x80u
#define STATUS_NO_REPORT    0x40u
#define STATUS_NO_TIMER     0x1u

struct TestResult {
    unsigned int status;
    unsigned int timer_low;
    unsigned int spin_iters;
    unsigned int value;
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
    volatile gcmReportData *report;
    unsigned int iters;
    u32 *cmd;
    s32 rc;

    if (sysMemoryAllocate(HOST_SIZE, SYS_MEMORY_PAGE_SIZE_1M, &host) != 0) {
        result.status |= STATUS_ALLOC_FAILED;
        write_tty_result(&result);
        return (int)result.status;
    }
    rc = rsxInit(&context, CMD_SIZE, HOST_SIZE, (void *)(unsigned long)host);
    if (rc != 0) {
        result.status |= STATUS_INIT_FAILED;
        result.timer_low = (unsigned int)rc;
        write_tty_result(&result);
        return (int)result.status;
    }
    report = gcmGetReportDataAddress(REPORT_INDEX);
    if (report == 0) {
        result.status |= STATUS_NO_REPORT;
        write_tty_result(&result);
        return (int)result.status;
    }
    report->timer = 0;
    report->value = 0;

    cmd = context->current;
    cmd[0] = ((u32)1 << NV_COUNT_SHIFT) | NV4097_GET_REPORT;
    cmd[1] = ((u32)GCM_ZPASS_PIXEL_CNT << 24) | (REPORT_INDEX << 4);
    context->current = cmd + 2;
    rsxFlushBuffer(context);

    for (iters = 0; iters < MAX_POLLS; iters++) {
        if (report->timer != 0)
            break;
    }
    result.spin_iters = iters;
    result.timer_low = (unsigned int)report->timer;
    result.value = report->value;
    if (report->timer == 0)
        result.status |= STATUS_NO_TIMER;

    write_tty_result(&result);
    return (int)result.status;
}
