/* PPU program: run the SPU program once on one raw SPU and read back
 * how it stopped, its interrupt status, what it stored and what reached
 * the buffer.
 *
 * 1. Zero the 4 KB buffer, create one raw SPU, and set its class 0 and
 *    class 2 interrupt masks to 0, so an MFC or SPU error is only
 *    latched in the interrupt status.
 * 2. Load the SPU image, and write the buffer's effective address to
 *    the word at local-store address 0x1F00.
 * 3. Write 0 to SPU_NextPC and 1 (run) to SPU_RunCntl, and poll
 *    SPU_Status until the SPU stops; stop it if it never does.
 * 4. Read SPU_Status, SPU_NextPC and the class 0 and class 2 interrupt
 *    statuses, writing each back to clear it, word 0 of the eight
 *    result slots, and the buffer words at +0 and +0x10.
 * 5. Destroy the raw SPU and write one CGOV frame.
 *
 * Payload layout (112 bytes, big-endian words):
 *   +0  step that failed (0 = none)
 *   +4  that step's return code
 *   +8  the two mask write rcs
 *   +16 SPU_Status          +20 SPU_NextPC
 *   +24 class 0 status (u64)
 *   +32 class 2 status (u64)
 *   +40 slot 0..7 word 0 (0xEEEEEEEE where never stored)
 *   +72 the buffer words at +0 and +0x10
 *   +80 eight zero words
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
#define SLOTS_AT 0x2000u
#define SLOTS 8
#define BUFFER_EA_AT 0x1F00u
#define MAX_POLLS 1000000u

struct Result {
    unsigned int failed_step;
    unsigned int failed_rc;
    unsigned int mask_set_rc[2];
    unsigned int status;
    unsigned int npc;
    u64 stat0;
    u64 stat2;
    unsigned int slot[SLOTS];
    unsigned int buffer[2];
    unsigned int pad[8];
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
static unsigned char buffer[4096] __attribute__((aligned(128)));

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

static unsigned int buffer_word(unsigned int offset)
{
    const unsigned char *p = buffer + offset;
    return ((unsigned int)p[0] << 24) | ((unsigned int)p[1] << 16)
        | ((unsigned int)p[2] << 8) | (unsigned int)p[3];
}

int main(void)
{
    int ret;
    sys_raw_spu_t spu;
    sysSpuImage image;
    unsigned int i;

    memset(&result, 0, sizeof(result));
    memset(buffer, 0, sizeof(buffer));

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
    sysSpuRawWriteLocalStorage(spu, BUFFER_EA_AT, (unsigned int)(unsigned long)buffer);

    result.status = run_from(spu, 0);
    result.npc = sysSpuRawReadProblemStorage(spu, SPU_NextPC);
    sysSpuRawGetIntStat(spu, 0, &result.stat0);
    sysSpuRawSetIntStat(spu, 0, result.stat0);
    sysSpuRawGetIntStat(spu, 2, &result.stat2);
    sysSpuRawSetIntStat(spu, 2, result.stat2);
    for (i = 0; i < SLOTS; i++)
        result.slot[i] = sysSpuRawReadLocalStorage(spu, SLOTS_AT + i * 16);
    result.buffer[0] = buffer_word(0);
    result.buffer[1] = buffer_word(0x10);

    sysSpuRawDestroy(spu);
    write_result(&result);
    return 0;
}
