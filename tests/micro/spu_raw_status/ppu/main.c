/* PPU program: run one raw SPU to a stop and read its state back.
 *
 * 1. Reserve one raw SPU and create it.
 * 2. Load the SPU image into its local store.
 * 3. Write the program's entry to SPU_NextPC and 1 (run) to
 *    SPU_RunCntl.
 * 4. Poll SPU_Status until the run bit clears.
 * 5. Read SPU_Status, SPU_NextPC and the class 0, 1 and 2 interrupt
 *    status, and write them as one CGOV frame.
 *
 * Payload layout (64 bytes, big-endian):
 *   +0  u32 step that failed (0 = none)
 *   +4  u32 that step's return code
 *   +8  u32 SPU_Status after the stop
 *   +12 u32 SPU_NextPC after the stop
 *   +16 u32 the entry point the loaded image reports
 *   +20 u32 status polls until the run bit cleared
 *   +24 u32 x3 the return code of each class's interrupt-status read
 *   +36 u32 zero
 *   +40 u64 x3 class 0, 1 and 2 interrupt status
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
/* The SPU_RunCntl value that starts the SPU. */
#define SPU_RUN 0x1u
/* The SPU program's _start: build.sh links it at local-store 0. */
#define SPU_ENTRY 0x0u
/* Status polls before the program gives up waiting for the stop. */
#define MAX_POLLS 10000000u

struct RawResult {
    unsigned int failed_step;
    unsigned int failed_rc;
    unsigned int spu_status;
    unsigned int spu_npc;
    unsigned int image_entry;
    unsigned int polls;
    unsigned int int_rc[3];
    unsigned int zero;
    unsigned long long int_status[3];
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_result(const struct RawResult *r)
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

static struct RawResult result __attribute__((aligned(128)));

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
    unsigned int polls = 0;
    unsigned int status;
    int class_id;

    memset(&result, 0, sizeof(result));

    /* Initialization fails once the process has done it; the create
     * below is the step that must succeed. */
    sysSpuInitialize(6, 1);

    ret = sysSpuRawCreate(&spu, NULL);
    if (ret != 0)
        return fail(1, ret);

    ret = CGOV_SPU_IMAGE_OPEN(&image, SPU_ELF_PATH);
    if (ret != 0)
        return fail(2, ret);

    ret = sysSpuRawImageLoad(spu, &image);
    if (ret != 0)
        return fail(3, ret);

    result.image_entry = image.entryPoint;
    sysSpuRawWriteProblemStorage(spu, SPU_NextPC, SPU_ENTRY);
    sysSpuRawWriteProblemStorage(spu, SPU_RunCtrl, SPU_RUN);

    do {
        status = sysSpuRawReadProblemStorage(spu, SPU_Status);
        polls++;
    } while ((status & SPU_STATUS_RUNNING) != 0 && polls < MAX_POLLS);
    if ((status & SPU_STATUS_RUNNING) != 0)
        return fail(4, (int)status);

    result.polls = polls;
    result.spu_status = sysSpuRawReadProblemStorage(spu, SPU_Status);
    result.spu_npc = sysSpuRawReadProblemStorage(spu, SPU_NextPC);
    for (class_id = 0; class_id < 3; class_id++)
        result.int_rc[class_id] = (unsigned int)sysSpuRawGetIntStat(
            spu, class_id, &result.int_status[class_id]);

    write_result(&result);
    sysSpuRawDestroy(spu);
    return 0;
}
