/* PPU program: run the SPU program once on one raw SPU and read back
 * how it stopped and the decrementer count each case fell by.
 *
 * 1. Create one raw SPU and set its class 0 and class 2 interrupt masks
 *    to 0, so an SPU error is only latched in the interrupt status.
 * 2. Load the SPU image into its local store.
 * 3. Write 0 to SPU_NextPC and 1 (run) to SPU_RunCntl, poll SPU_Status
 *    until the SPU stops, read SPU_Status, SPU_NextPC and the class 0
 *    interrupt status, and read word 0 of the eight result slots.
 * 4. Destroy the raw SPU and write one CGOV frame.
 *
 * Payload layout (64 bytes, big-endian words):
 *   +0  step that failed (0 = none)
 *   +4  that step's return code
 *   +8  the two mask write rcs
 *   +16 SPU_Status
 *   +20 SPU_NextPC
 *   +24 class 0 status (u64)
 *   +32 slot 0..7 word 0: the decrementer count case k fell by
 *       (0xEEEEEEEE where never stored)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SPU_STATUS_RUNNING 0x1u
#define SPU_STOP 0x0u
#define SPU_RUN 0x1u
#define SLOTS_AT 0x8000u
#define SLOTS 8
#define MAX_POLLS 10000000u

struct Result {
    unsigned int failed_step;
    unsigned int failed_rc;
    unsigned int mask_set_rc[2];
    unsigned int status;
    unsigned int npc;
    u64 stat;
    unsigned int slot[SLOTS];
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_result(const struct Result *r)
{
    unsigned int written;
    unsigned int len = sizeof(*r);
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, r, len, &written);
    CGOV_OUT_FILE_WRITE(r, len);
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

static struct Result result __attribute__((aligned(128)));

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    result.failed_step = step;
    result.failed_rc = (unsigned int)rc;
    write_result(&result);
    return (int)step;
}

int main(void)
{
    int ret;
    sys_raw_spu_t spu;
    sysSpuImage image;
    unsigned int i;
    unsigned int status;
    unsigned int polls = 0;

    memset(&result, 0, sizeof(result));

    /* Initialization fails once the process has done it; the create
     * below is the step that must succeed. */
    sysSpuInitialize(6, 1);

    ret = sysSpuRawCreate(&spu, NULL);
    if (ret != 0)
        return fail(1, ret);

    for (i = 0; i < 2; i++)
        result.mask_set_rc[i] = (unsigned int)sysSpuRawSetIntMask(spu, i * 2, 0);

    ret = CGOV_SPU_IMAGE_OPEN(&image, SPU_ELF_PATH);
    if (ret != 0) {
        sysSpuRawDestroy(spu);
        return fail(2, ret);
    }

    ret = sysSpuRawImageLoad(spu, &image);
    if (ret != 0) {
        sysSpuRawDestroy(spu);
        return fail(3, ret);
    }

    sysSpuRawWriteProblemStorage(spu, SPU_NextPC, 0);
    sysSpuRawWriteProblemStorage(spu, SPU_RunCtrl, SPU_RUN);
    /* A read right after the run write can come before the run bit
     * shows, so wait for a stop reason, not just for the run bit to
     * clear. */
    do {
        status = sysSpuRawReadProblemStorage(spu, SPU_Status);
        polls++;
    } while (((status & SPU_STATUS_RUNNING) != 0 || status == 0) && polls < MAX_POLLS);
    if ((status & SPU_STATUS_RUNNING) != 0)
        sysSpuRawWriteProblemStorage(spu, SPU_RunCtrl, SPU_STOP);
    result.status = sysSpuRawReadProblemStorage(spu, SPU_Status);
    result.npc = sysSpuRawReadProblemStorage(spu, SPU_NextPC);
    sysSpuRawGetIntStat(spu, 0, &result.stat);
    sysSpuRawSetIntStat(spu, 0, result.stat);
    for (i = 0; i < SLOTS; i++)
        result.slot[i] = sysSpuRawReadLocalStorage(spu, SLOTS_AT + i * 16);

    sysSpuRawDestroy(spu);
    write_result(&result);
    return 0;
}
