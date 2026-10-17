/* PPU program: run each of the SPU program's 2048 blocks on one raw SPU
 * and read back how it stopped.
 *
 * 1. Create one raw SPU and set its class 0 and class 2 interrupt masks
 *    to 0, so an SPU error is only latched in the interrupt status.
 * 2. Fill the whole local store with `stop 0x3FF0`, then load the SPU
 *    image over it.
 * 3. For block p = 0..2047: write 0x1000 + 16 p to SPU_NextPC and 1 (run)
 *    to SPU_RunCntl, poll SPU_Status until the SPU stops (stopping it if
 *    it never does), and read SPU_Status, SPU_NextPC and the class 0
 *    interrupt status, writing that status back to clear it.
 * 4. Destroy the raw SPU and write one CGOV frame.
 *
 * Payload layout (24592 bytes, big-endian words):
 *   w0  step that failed (0 = none)   w1  that step's return code
 *   w2  the two mask write rcs (w2, w3)
 *   w4  block p at w4 + 3 p: SPU_Status, SPU_NextPC, class 0 status
 *       low word
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
#define BLOCKS_AT 0x1000u
#define BLOCK 16u
#define PREFIXES 2048
#define LS_BYTES 0x40000u
#define FILL_WORD 0x00003FF0u
#define MAX_POLLS 1000000u

struct Record {
    unsigned int status;
    unsigned int npc;
    unsigned int stat0;
};

struct Result {
    unsigned int failed_step;
    unsigned int failed_rc;
    unsigned int mask_set_rc[2];
    struct Record block[PREFIXES];
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
    unsigned int p, i;

    memset(&result, 0, sizeof(result));

    /* Initialization fails once the process has done it; the create
     * below is the step that must succeed. */
    sysSpuInitialize(6, 1);

    ret = sysSpuRawCreate(&spu, NULL);
    if (ret != 0)
        return fail(1, ret);
    for (i = 0; i < 2; i++)
        result.mask_set_rc[i] = (unsigned int)sysSpuRawSetIntMask(spu, i * 2, 0);

    for (i = 0; i < LS_BYTES; i += 4)
        sysSpuRawWriteLocalStorage(spu, i, FILL_WORD);

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

    for (p = 0; p < PREFIXES; p++) {
        struct Record *rec = &result.block[p];
        u64 stat = 0;

        rec->status = run_from(spu, BLOCKS_AT + p * BLOCK);
        rec->npc = sysSpuRawReadProblemStorage(spu, SPU_NextPC);
        sysSpuRawGetIntStat(spu, 0, &stat);
        sysSpuRawSetIntStat(spu, 0, stat);
        rec->stat0 = (unsigned int)stat;
    }

    sysSpuRawDestroy(spu);
    write_result(&result);
    return 0;
}
