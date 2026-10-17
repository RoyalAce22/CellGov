/* PPU program: run each reserved-channel case of the SPU program on one
 * raw SPU, then its storage control commands, and read back how each run
 * stopped, what it stored and what the buffer holds.
 *
 * 1. Create one raw SPU, set its class 0 and class 2 interrupt masks to
 *    0, load the SPU image, and write the buffer's effective address to
 *    local-store address 0x1F00. Word j of buffer line k is
 *    0xA0000000 | k << 8 | j.
 * 2. For case n = 0..15: run from 0x20 n, polling SPU_Status until the
 *    SPU stops (stopping it if it never does). Record SPU_Status,
 *    SPU_NextPC, slot n and the class 0 and class 2 status low words,
 *    writing each status back to clear it.
 * 3. Run from 0x400 the same way and record the same, the five tag
 *    statuses, and for buffer lines 0..5 word 0 and the number of
 *    nonzero words.
 * 4. Destroy the raw SPU and write one CGOV frame.
 *
 * Payload layout (448 bytes, big-endian words):
 *   w0   step that failed (0 = none)   w1  that step's return code
 *   w2   the two mask write rcs (w2, w3)
 *   w4   case n at w4 + 5 n: SPU_Status, SPU_NextPC, slot n word 0,
 *        class 0 status low word, class 2 status low word (w4..w83)
 *   w84  SPU_Status after the storage control run   w85 SPU_NextPC
 *   w86  class 0 status low word                     w87 class 2
 *   w88  the tag status after command k (w88 + k, k = 0..4)
 *   w93  line k at w93 + 2 k: word 0, nonzero words (k = 0..5)
 *   w105 seven zero words
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
#define MAX_POLLS 1000000u

#define CASES 16u
#define CASE_STRIDE 0x20u
#define SLOTS_AT 0x2000u
#define SL1_AT 0x400u
#define TAG_STAT_LS 0x2200u
#define COMMANDS 5u
#define LINES 6u
#define LINE 0x80u

struct Case {
    unsigned int status;
    unsigned int npc;
    unsigned int slot;
    unsigned int stat0;
    unsigned int stat2;
};

struct Result {
    unsigned int failed_step;
    unsigned int failed_rc;
    unsigned int mask_set_rc[2];
    struct Case cases[CASES];
    unsigned int sl1_status;
    unsigned int sl1_npc;
    unsigned int sl1_stat0;
    unsigned int sl1_stat2;
    unsigned int tag_stat[COMMANDS];
    unsigned int line[LINES][2];
    unsigned int pad[7];
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

/* Read and clear the class 0 and class 2 statuses; return their low
 * words. */
static void clear_stats(sys_raw_spu_t spu, unsigned int *stat0, unsigned int *stat2)
{
    u64 s0, s2;

    sysSpuRawGetIntStat(spu, 0, &s0);
    sysSpuRawSetIntStat(spu, 0, s0);
    sysSpuRawGetIntStat(spu, 2, &s2);
    sysSpuRawSetIntStat(spu, 2, s2);
    *stat0 = (unsigned int)s0;
    *stat2 = (unsigned int)s2;
}

static unsigned int buffer_word(unsigned int offset)
{
    const unsigned char *p = buffer + offset;
    return ((unsigned int)p[0] << 24) | ((unsigned int)p[1] << 16)
        | ((unsigned int)p[2] << 8) | (unsigned int)p[3];
}

static void set_buffer_word(unsigned int offset, unsigned int value)
{
    unsigned char *p = buffer + offset;
    p[0] = (value >> 24) & 0xFF;
    p[1] = (value >> 16) & 0xFF;
    p[2] = (value >> 8) & 0xFF;
    p[3] = value & 0xFF;
}

int main(void)
{
    int ret;
    sys_raw_spu_t spu;
    sysSpuImage image;
    unsigned int i, j;

    memset(&result, 0, sizeof(result));
    memset(buffer, 0, sizeof(buffer));
    for (i = 0; i < LINES; i++)
        for (j = 0; j < LINE / 4; j++)
            set_buffer_word(i * LINE + j * 4, 0xA0000000u | i << 8 | j);

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

    for (i = 0; i < CASES; i++) {
        struct Case *c = &result.cases[i];
        c->status = run_from(spu, i * CASE_STRIDE);
        c->npc = sysSpuRawReadProblemStorage(spu, SPU_NextPC);
        clear_stats(spu, &c->stat0, &c->stat2);
        c->slot = sysSpuRawReadLocalStorage(spu, SLOTS_AT + i * 16);
    }

    result.sl1_status = run_from(spu, SL1_AT);
    result.sl1_npc = sysSpuRawReadProblemStorage(spu, SPU_NextPC);
    clear_stats(spu, &result.sl1_stat0, &result.sl1_stat2);
    for (i = 0; i < COMMANDS; i++)
        result.tag_stat[i] = sysSpuRawReadLocalStorage(spu, TAG_STAT_LS + i * 16);
    for (i = 0; i < LINES; i++) {
        unsigned int nonzero = 0;
        for (j = 0; j < LINE / 4; j++)
            nonzero += buffer_word(i * LINE + j * 4) != 0;
        result.line[i][0] = buffer_word(i * LINE);
        result.line[i][1] = nonzero;
    }

    sysSpuRawDestroy(spu);
    write_result(&result);
    return 0;
}
