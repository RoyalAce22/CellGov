/* PPU program: run the three parts of the SPU program on one raw SPU and
 * drain its two outbound mailboxes between them.
 *
 * 1. Create one raw SPU, set its class 0 and class 2 interrupt masks to
 *    0, and load the SPU image.
 * 2. Run from 0x000, then read SPU_Out_Mbox.
 * 3. Run from 0x100, then read SPU_Out_Intr_Mbox (sys_raw_spu_read_puint_mb).
 * 4. Run from 0x200.
 * 5. Read the SPU's words, destroy the raw SPU, and write one CGOV frame.
 * After every run the class 0 and class 2 statuses are read and written
 * back to clear them.
 *
 * Payload layout (96 bytes, big-endian words):
 *   w0  step that failed (0 = none)    w1  that step's return code
 *   w2  the two mask write rcs (w2, w3)
 *   w4  SPU_Status after each run (w4, w5, w6)
 *   w7  the SPU_Out_Mbox word          w8  the puint read rc
 *   w9  the SPU_Out_Intr_Mbox word
 *   w10 class 2 status low word after the second run
 *   w11 the SPU's words at 0x2000 + 16 k for k = 0..10 (w11..w21; the
 *       SPU program's header comment names them)
 *   w22 0                              w23 0
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
#define MAX_POLLS 1000000u

#define PS_OUT_MBOX 0x4004u

#define WORDS_LS 0x2000u
#define SPU_WORDS 11u

struct Result {
    unsigned int failed_step;
    unsigned int failed_rc;
    unsigned int mask_set_rc[2];
    unsigned int status[3];
    unsigned int out_word;
    unsigned int puint_rc;
    unsigned int puint_word;
    unsigned int stat2_after_intr;
    unsigned int spu_word[SPU_WORDS];
    unsigned int pad[2];
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

/* Read and clear the class 0 and class 2 statuses. */
static void clear_stats(sys_raw_spu_t spu, u64 *stat0, u64 *stat2)
{
    sysSpuRawGetIntStat(spu, 0, stat0);
    sysSpuRawSetIntStat(spu, 0, *stat0);
    sysSpuRawGetIntStat(spu, 2, stat2);
    sysSpuRawSetIntStat(spu, 2, *stat2);
}

int main(void)
{
    int ret;
    sys_raw_spu_t spu;
    sysSpuImage image;
    unsigned int i;
    u64 stat0, stat2;
    u32 word;

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

    result.status[0] = run_from(spu, 0x000);
    clear_stats(spu, &stat0, &stat2);
    result.out_word = sysSpuRawReadProblemStorage(spu, PS_OUT_MBOX);

    result.status[1] = run_from(spu, 0x100);
    clear_stats(spu, &stat0, &stat2);
    result.stat2_after_intr = (unsigned int)stat2;
    word = 0;
    result.puint_rc = (unsigned int)sysSpuRawReadPuintMb(spu, &word);
    result.puint_word = (unsigned int)word;

    result.status[2] = run_from(spu, 0x200);
    clear_stats(spu, &stat0, &stat2);

    for (i = 0; i < SPU_WORDS; i++)
        result.spu_word[i] = sysSpuRawReadLocalStorage(spu, WORDS_LS + i * 16);

    sysSpuRawDestroy(spu);
    write_result(&result);
    return 0;
}
