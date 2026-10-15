/* PPU program: run each of the SPU program's six entry points on one
 * raw SPU and read back how it stopped.
 *
 * 1. Create one raw SPU, read its class 0 and class 2 interrupt masks,
 *    and set both to 0, so an SPU error is only latched in the
 *    interrupt status and no interrupt reaches the kernel.
 * 2. Load the SPU image into its local store.
 * 3. For entry k = 0..5: write the entry's local-store address to
 *    SPU_NextPC and 1 (run) to
 *    SPU_RunCntl, poll SPU_Status until the run bit clears with a stop
 *    reason set, read
 *    SPU_Status, SPU_NextPC and the class 0 interrupt status, write
 *    that status back to clear it, and read result slot k from the
 *    local store.
 * 4. Destroy the raw SPU and write one CGOV frame.
 *
 * Payload layout (336 bytes, big-endian words):
 *   +0  step that failed (0 = none)
 *   +4  that step's return code
 *   +8  the image's entry point
 *   +12 zero
 *   +16 class 0 interrupt mask before step 1 set it (u64)
 *   +24 class 2 interrupt mask before step 1 set it (u64)
 *   +32 the two mask read rcs, then the two mask write rcs
 *   +48 six records of 12 words (48 bytes), one per entry:
 *         +0  SPU_Status          +4  SPU_NextPC
 *         +8  polls until the run bit cleared
 *         +12 class 0 status read rc
 *         +16 class 0 status (u64)
 *         +24 class 0 status clear rc
 *         +28 zero
 *         +32 result slot k (4 words; 0xEEEEEEEE where never stored)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

/* The run bit of SPU_Status. */
#define SPU_STATUS_RUNNING 0x1u
/* The SPU_RunCntl values that stop and start the SPU. */
#define SPU_STOP 0x0u
#define SPU_RUN 0x1u
/* The SPU program's _start: build.sh links it at local-store 0. The
 * image's reported entry is recorded but is not a local-store address. */
#define SPU_START 0x0u
/* Entry points are this far apart, from _start. */
#define ENTRY_STRIDE 0x40u
#define ENTRIES 6
/* Result slot k sits at _start plus this, 16 bytes per slot. */
#define RESULTS_AT 0x300u
/* Status polls before the program stops waiting. */
#define MAX_POLLS 1000000u

struct Record {
    unsigned int status;
    unsigned int npc;
    unsigned int polls;
    unsigned int stat_rc;
    u64 stat;
    unsigned int clear_rc;
    unsigned int zero;
    unsigned int result[4];
};

struct Result {
    unsigned int failed_step;
    unsigned int failed_rc;
    unsigned int image_entry;
    unsigned int zero;
    u64 mask[2];
    unsigned int mask_get_rc[2];
    unsigned int mask_set_rc[2];
    struct Record entry[ENTRIES];
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
    unsigned int k, i;

    memset(&result, 0, sizeof(result));

    /* Initialization fails once the process has done it; the create
     * below is the step that must succeed. */
    sysSpuInitialize(6, 1);

    ret = sysSpuRawCreate(&spu, NULL);
    if (ret != 0)
        return fail(1, ret);

    for (i = 0; i < 2; i++) {
        result.mask_get_rc[i] = (unsigned int)sysSpuRawGetIntMask(spu, i * 2, &result.mask[i]);
        result.mask_set_rc[i] = (unsigned int)sysSpuRawSetIntMask(spu, i * 2, 0);
    }

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
    result.image_entry = image.entryPoint;

    for (k = 0; k < ENTRIES; k++) {
        struct Record *rec = &result.entry[k];
        unsigned int entry = SPU_START + k * ENTRY_STRIDE;
        unsigned int status;
        unsigned int polls = 0;

        sysSpuRawWriteProblemStorage(spu, SPU_NextPC, entry);
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

        rec->polls = polls;
        rec->status = sysSpuRawReadProblemStorage(spu, SPU_Status);
        rec->npc = sysSpuRawReadProblemStorage(spu, SPU_NextPC);
        rec->stat_rc = (unsigned int)sysSpuRawGetIntStat(spu, 0, &rec->stat);
        rec->clear_rc = (unsigned int)sysSpuRawSetIntStat(spu, 0, rec->stat);
        for (i = 0; i < 4; i++)
            rec->result[i] = sysSpuRawReadLocalStorage(
                spu, SPU_START + RESULTS_AT + k * 16 + i * 4);
    }

    sysSpuRawDestroy(spu);
    write_result(&result);
    return 0;
}
