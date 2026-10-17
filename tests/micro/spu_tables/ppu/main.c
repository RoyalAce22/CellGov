/* PPU program: run the four parts of the SPU program on one raw SPU and
 * read back the channel counts, the mailbox state and the MFC results.
 *
 * 1. Create one raw SPU, set its class 0 and class 2 interrupt masks to
 *    0, load the SPU image, and write the buffer's effective address to
 *    local-store address 0x1F00. Fill line k of the 4 KB buffer with
 *    0xA000000k in its first word.
 * 2. The channel sweep: run from 0. Each time the SPU stops on anything
 *    but stop 0x2001, record the block (NPC / 8), SPU_Status, SPU_NextPC
 *    and the class 0 status, and run again from the next block.
 * 3. The in mailbox: write 0x1571000k to SPU_In_Mbox for k = 1..5,
 *    reading SPU_Mbox_Stat after each, then run from 0x800.
 * 4. The out mailbox: read SPU_Mbox_Stat, run from 0x900, read
 *    SPU_Mbox_Stat, SPU_Out_Mbox, and SPU_Mbox_Stat again.
 * 5. The MFC commands: run from 0xA00.
 * 6. Destroy the raw SPU and write one CGOV frame.
 * After every run the class 0 and class 2 statuses are read and written
 * back to clear them.
 *
 * Payload layout (1024 bytes, big-endian words):
 *   w0   step that failed (0 = none)   w1  that step's return code
 *   w2   the two mask write rcs (w2, w3)
 *   w4   SPU_Status after the sweep    w5  restarts in the sweep
 *   w6   eight restart records of four words: block, SPU_Status,
 *        SPU_NextPC, class 0 status low word (w6..w37)
 *   w38  rchcnt of channel n at w38 + n (n = 0..127; 0xEEEEEEEE where
 *        never stored)
 *   w166 SPU_Mbox_Stat after in-mailbox writes 1..5 (w166..w170)
 *   w171 SPU_Status after the in-mailbox run
 *   w172 rchcnt 29 before the reads   w173..w176 the four words read
 *   w177 rchcnt 29 after the reads    w178 SPU_Mbox_Stat after the run
 *   w179 SPU_Mbox_Stat before the out-mailbox run
 *   w180 SPU_Status after it          w181 rchcnt 28 before the write
 *   w182 rchcnt 28 after the write    w183 SPU_Mbox_Stat after the run
 *   w184 SPU_Out_Mbox                 w185 SPU_Mbox_Stat after that read
 *   w186 SPU_Status after the MFC run w187 SPU_NextPC
 *   w188 class 0 status (u64)         w190 class 2 status (u64)
 *   w192 tag status after command k (w192 + k; 0xEEEEEEEE where none)
 *   w208 atomic status after command k (w208 + k)
 *   w224 local-store line k word 0 (w224 + k)
 *   w240 buffer line k word 0 (w240 + k)
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

#define PS_OUT_MBOX 0x4004u
#define PS_IN_MBOX 0x400Cu
#define PS_MBOX_STAT 0x4014u

#define SWEEP_AT 0x0u
#define SWEEP_BLOCKS 128u
#define SWEEP_DONE 0x2001u
#define MBOX_IN_AT 0x800u
#define MBOX_OUT_AT 0x900u
#define MFC_AT 0xA00u
#define COUNTS_AT 0x2000u
#define MBOX_IN_LS 0x2800u
#define MBOX_OUT_LS 0x2880u
#define TAG_STAT_LS 0x3000u
#define ATOMIC_STAT_LS 0x3100u
#define LINES_LS 0x4000u
#define LINES 16u
#define MAX_RESTARTS 8u

struct Result {
    unsigned int failed_step;
    unsigned int failed_rc;
    unsigned int mask_set_rc[2];
    unsigned int sweep_status;
    unsigned int restarts;
    unsigned int restart[MAX_RESTARTS][4];
    unsigned int count[SWEEP_BLOCKS];
    unsigned int in_stat[5];
    unsigned int in_status;
    unsigned int in_count_before;
    unsigned int in_word[4];
    unsigned int in_count_after;
    unsigned int in_stat_after;
    unsigned int out_stat_before;
    unsigned int out_status;
    unsigned int out_count_before;
    unsigned int out_count_after;
    unsigned int out_stat_after_run;
    unsigned int out_word;
    unsigned int out_stat_after_read;
    unsigned int mfc_status;
    unsigned int mfc_npc;
    u64 stat0;
    u64 stat2;
    unsigned int tag_stat[LINES];
    unsigned int atomic_stat[LINES];
    unsigned int ls_line[LINES];
    unsigned int ea_line[LINES];
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

/* Read and clear the class 0 and class 2 statuses. */
static void clear_stats(sys_raw_spu_t spu, u64 *stat0, u64 *stat2)
{
    sysSpuRawGetIntStat(spu, 0, stat0);
    sysSpuRawSetIntStat(spu, 0, *stat0);
    sysSpuRawGetIntStat(spu, 2, stat2);
    sysSpuRawSetIntStat(spu, 2, *stat2);
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

static void sweep(sys_raw_spu_t spu)
{
    unsigned int entry = SWEEP_AT;
    unsigned int status = 0;
    unsigned int runs;
    u64 stat0, stat2;

    for (runs = 0; runs <= SWEEP_BLOCKS; runs++) {
        unsigned int npc, block;

        status = run_from(spu, entry);
        npc = sysSpuRawReadProblemStorage(spu, SPU_NextPC);
        clear_stats(spu, &stat0, &stat2);
        if ((status >> 16) == SWEEP_DONE)
            break;
        block = npc / 8;
        if (result.restarts < MAX_RESTARTS) {
            unsigned int *rec = result.restart[result.restarts];
            rec[0] = block;
            rec[1] = status;
            rec[2] = npc;
            rec[3] = (unsigned int)stat0;
        }
        result.restarts++;
        if (block + 1 >= SWEEP_BLOCKS)
            break;
        entry = SWEEP_AT + (block + 1) * 8;
    }
    result.sweep_status = status;
}

int main(void)
{
    int ret;
    sys_raw_spu_t spu;
    sysSpuImage image;
    unsigned int i;
    u64 stat0, stat2;

    memset(&result, 0, sizeof(result));
    memset(buffer, 0, sizeof(buffer));
    for (i = 0; i < LINES; i++)
        set_buffer_word(i * 0x80, 0xA0000000u | i);

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

    sweep(spu);
    for (i = 0; i < SWEEP_BLOCKS; i++)
        result.count[i] = sysSpuRawReadLocalStorage(spu, COUNTS_AT + i * 16);

    for (i = 0; i < 5; i++) {
        sysSpuRawWriteProblemStorage(spu, PS_IN_MBOX, 0x15710000u | (i + 1));
        result.in_stat[i] = sysSpuRawReadProblemStorage(spu, PS_MBOX_STAT);
    }
    result.in_status = run_from(spu, MBOX_IN_AT);
    clear_stats(spu, &stat0, &stat2);
    result.in_count_before = sysSpuRawReadLocalStorage(spu, MBOX_IN_LS);
    for (i = 0; i < 4; i++)
        result.in_word[i] = sysSpuRawReadLocalStorage(spu, MBOX_IN_LS + 0x10 + i * 16);
    result.in_count_after = sysSpuRawReadLocalStorage(spu, MBOX_IN_LS + 0x50);
    result.in_stat_after = sysSpuRawReadProblemStorage(spu, PS_MBOX_STAT);

    result.out_stat_before = sysSpuRawReadProblemStorage(spu, PS_MBOX_STAT);
    result.out_status = run_from(spu, MBOX_OUT_AT);
    clear_stats(spu, &stat0, &stat2);
    result.out_count_before = sysSpuRawReadLocalStorage(spu, MBOX_OUT_LS);
    result.out_count_after = sysSpuRawReadLocalStorage(spu, MBOX_OUT_LS + 0x10);
    result.out_stat_after_run = sysSpuRawReadProblemStorage(spu, PS_MBOX_STAT);
    result.out_word = sysSpuRawReadProblemStorage(spu, PS_OUT_MBOX);
    result.out_stat_after_read = sysSpuRawReadProblemStorage(spu, PS_MBOX_STAT);

    result.mfc_status = run_from(spu, MFC_AT);
    result.mfc_npc = sysSpuRawReadProblemStorage(spu, SPU_NextPC);
    clear_stats(spu, &result.stat0, &result.stat2);
    for (i = 0; i < LINES; i++) {
        result.tag_stat[i] = sysSpuRawReadLocalStorage(spu, TAG_STAT_LS + i * 16);
        result.atomic_stat[i] = sysSpuRawReadLocalStorage(spu, ATOMIC_STAT_LS + i * 16);
        result.ls_line[i] = sysSpuRawReadLocalStorage(spu, LINES_LS + i * 0x80);
        result.ea_line[i] = buffer_word(i * 0x80);
    }

    sysSpuRawDestroy(spu);
    write_result(&result);
    return 0;
}
