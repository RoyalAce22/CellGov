/* PPU program: run the SPU program once on one raw SPU and read back
 * whether the transfers it queued before it stopped reach the buffer.
 *
 * 1. Zero the 64 KB buffer, create one raw SPU, and set its class 0 and
 *    class 2 interrupt masks to 0.
 * 2. Load the SPU image, and write the buffer's effective address to
 *    the word at local-store address 0x1F00.
 * 3. Write 0 to SPU_NextPC and 1 (run) to SPU_RunCntl, and poll
 *    SPU_Status until the SPU stops; stop it if it never does.
 * 4. At once, read MFC_QStatus, the proxy tag status for tag 1 (an
 *    immediate query) and the last word of each 16 KB of the buffer.
 * 5. Read SPU_Status 1000000 more times, then read the same values
 *    again, and the first word of each 16 KB.
 * 6. Destroy the raw SPU and write one CGOV frame.
 *
 * Payload layout (96 bytes, big-endian words):
 *   +0  step that failed (0 = none)
 *   +4  that step's return code
 *   +8  the two mask write rcs
 *   +16 SPU_Status          +20 SPU_NextPC
 *   +24 MFC_QStatus at the stop      +28 tag status at the stop
 *   +32 last word of each 16 KB at the stop (4 words)
 *   +48 MFC_QStatus later            +52 tag status later
 *   +56 last word of each 16 KB later (4 words)
 *   +72 first word of each 16 KB later (4 words)
 *   +88 two zero words
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
#define BUFFER_EA_AT 0x1F00u
#define CHUNK 0x4000u
#define CHUNKS 4
#define MAX_POLLS 1000000u
#define LATER_READS 1000000u
#define TAG1 0x2u
#define QUERY_IMMEDIATE 0u

struct Result {
    unsigned int failed_step;
    unsigned int failed_rc;
    unsigned int mask_set_rc[2];
    unsigned int status;
    unsigned int npc;
    unsigned int qstatus_at_stop;
    unsigned int tag_at_stop;
    unsigned int last_at_stop[CHUNKS];
    unsigned int qstatus_later;
    unsigned int tag_later;
    unsigned int last_later[CHUNKS];
    unsigned int first_later[CHUNKS];
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
static volatile unsigned int buffer[CHUNKS * CHUNK / 4] __attribute__((aligned(128)));

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

/* The proxy tag status of tag 1, by an immediate query. */
static unsigned int tag_status(sys_raw_spu_t spu)
{
    sysSpuRawWriteProblemStorage(spu, Prxy_QueryMask, TAG1);
    sysSpuRawWriteProblemStorage(spu, Prxy_QueryType, QUERY_IMMEDIATE);
    return sysSpuRawReadProblemStorage(spu, Prxy_TagStatus);
}

int main(void)
{
    int ret;
    sys_raw_spu_t spu;
    sysSpuImage image;
    unsigned int i;

    memset(&result, 0, sizeof(result));
    for (i = 0; i < CHUNKS * CHUNK / 4; i++)
        buffer[i] = 0;

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
    result.qstatus_at_stop = sysSpuRawReadProblemStorage(spu, MFC_QStatus);
    result.tag_at_stop = tag_status(spu);
    for (i = 0; i < CHUNKS; i++)
        result.last_at_stop[i] = buffer[(i + 1) * CHUNK / 4 - 1];

    for (i = 0; i < LATER_READS; i++)
        (void)sysSpuRawReadProblemStorage(spu, SPU_Status);

    result.npc = sysSpuRawReadProblemStorage(spu, SPU_NextPC);
    result.qstatus_later = sysSpuRawReadProblemStorage(spu, MFC_QStatus);
    result.tag_later = tag_status(spu);
    for (i = 0; i < CHUNKS; i++) {
        result.last_later[i] = buffer[(i + 1) * CHUNK / 4 - 1];
        result.first_later[i] = buffer[i * CHUNK / 4];
    }

    sysSpuRawDestroy(spu);
    write_result(&result);
    return 0;
}
