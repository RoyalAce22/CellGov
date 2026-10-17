/* PPU program: run each of the SPU program's seventy-five cases on one
 * raw SPU and read back where it resumes after the invalid word.
 *
 * 1. Create one raw SPU and set its class 0 and class 2 interrupt masks
 *    to 0, so an SPU error is only latched in the interrupt status.
 * 2. Load the SPU image into its local store.
 * 3. For case k = 0..74: write k * 0x40 to SPU_NextPC and 1 (run) to
 *    SPU_RunCntl, poll SPU_Status until the SPU stops, read SPU_Status,
 *    SPU_NextPC and the class 0 interrupt status, write that status back
 *    to clear it, and read the first word of mark k. Then run the dump
 *    routine at 0x1800 the same way and read the first word of the dump
 *    slot: the value of $6 the case left. Then run the case a second
 *    time and read SPU_Status and SPU_NextPC again.
 * 4. Destroy the raw SPU and write one CGOV frame.
 *
 * Payload layout (2416 bytes, big-endian words):
 *   +0  step that failed (0 = none)
 *   +4  that step's return code
 *   +8  the two mask write rcs
 *   +16 seventy-five records of 8 words (32 bytes), one per case:
 *         +0  SPU_Status          +4  SPU_NextPC
 *         +8  class 0 status (u64)
 *         +16 mark k word 0 (0x77 when the case's stqr ran, else 0)
 *         +20 $6 word 0 from the dump (1 when the case's ai ran, else
 *             0); bit 31 set when the dump did not stop on 0x3FFE
 *         +24 SPU_Status          +28 SPU_NextPC, from the second run
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
#define CASE_STRIDE 0x40u
#define CASES 75
#define MARKS 0x2000u
#define DUMP 0x1800u
#define DUMP_SLOT 0x1900u
/* SPU_Status of a stop-and-signal with code 0x3FFE: the code in the
 * upper half and P (0x2). */
#define DUMP_STATUS 0x3FFE0002u
#define MAX_POLLS 1000000u

struct Record {
    unsigned int status;
    unsigned int npc;
    u64 stat;
    unsigned int mark;
    unsigned int r6;
    unsigned int status2;
    unsigned int npc2;
};

struct Result {
    unsigned int failed_step;
    unsigned int failed_rc;
    unsigned int mask_set_rc[2];
    struct Record entry[CASES];
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

/* Run the SPU from `entry` until it stops; return SPU_Status. */
static unsigned int run_from(sys_raw_spu_t spu, unsigned int entry)
{
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
    return sysSpuRawReadProblemStorage(spu, SPU_Status);
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

    for (k = 0; k < CASES; k++) {
        struct Record *rec = &result.entry[k];
        u64 stat = 0;

        rec->status = run_from(spu, k * CASE_STRIDE);
        rec->npc = sysSpuRawReadProblemStorage(spu, SPU_NextPC);
        sysSpuRawGetIntStat(spu, 0, &rec->stat);
        sysSpuRawSetIntStat(spu, 0, rec->stat);
        rec->mark = sysSpuRawReadLocalStorage(spu, MARKS + k * 16);

        if (run_from(spu, DUMP) != DUMP_STATUS)
            rec->r6 = 0x80000000u;
        sysSpuRawGetIntStat(spu, 0, &stat);
        sysSpuRawSetIntStat(spu, 0, stat);
        rec->r6 |= sysSpuRawReadLocalStorage(spu, DUMP_SLOT);

        rec->status2 = run_from(spu, k * CASE_STRIDE);
        rec->npc2 = sysSpuRawReadProblemStorage(spu, SPU_NextPC);
        sysSpuRawGetIntStat(spu, 0, &stat);
        sysSpuRawSetIntStat(spu, 0, stat);
    }

    sysSpuRawDestroy(spu);
    write_result(&result);
    return 0;
}
