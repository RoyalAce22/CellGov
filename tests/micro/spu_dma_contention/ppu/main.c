/* PPU program: run the DMA contention SPU program in six SPU thread
 * groups, one to six threads, and write back what thread 0 measured.
 *
 * For k = 0 to 5 streamers: clear the control line, create a group of
 * k + 1 threads (thread i gets arg0 = i << 32 | k, arg1 = the result
 * buffer, arg2 = the data region), start it, join it and destroy it. A
 * group the console refuses is recorded and the next one runs.
 *
 * Timing words are sums in decrementer ticks (the time-base frequency).
 * "Queue" times N transfers issued back to back under one tag, from the
 * first issue to the end of the tag wait. A 16 KB transfer reuses one
 * 16 KB buffer; a 1 KB one has its own slice of a 64 KB region.
 *
 * Payload layout (384 bytes, big-endian words):
 *   0   step that failed (0 = none)       1   that step's return code
 *   2 + 2k, 3 + 2k  group k's join cause and status (k = 0..5)
 *   14, 15  zero
 *   The group with no streamer:
 *   16  REPS (32)   17  REPS_Q (8)   18  a tag wait with nothing queued
 *   19  zero
 *   20 + i  queue of N 1 KB gets     25 + i  queue of N 16 KB gets
 *   30 + i  queue of N 1 KB puts     35 + i  queue of N 16 KB puts
 *   40 + i  getl of N 128 B          45 + i  getl of N 1 KB
 *   50 + i  getl of N 4 KB           55 + i  putl of N 1 KB
 *           (N = 1, 2, 4, 8, 16 for i = 0..4; REPS_Q runs each)
 *   60  128 rounds of 64 quadword loads, alone (REPS_Q runs)
 *   61, 62  the loads beside sixteen 4 KB gets queued first, and the
 *           time from the loop's start to the end of the gets' tag wait
 *   63  the same rounds of stores, alone
 *   64, 65  the stores beside sixteen 4 KB gets, and the wait's end
 *   66, 67  the loads beside sixteen 4 KB puts, and the wait's end
 *   68, 69  the loads beside sixteen 1 KB gets, and the wait's end
 *   70, 71  zero
 *   72 + 4k  group k: one 16 KB get (REPS runs), sixteen 4 KB gets in
 *            one queue (REPS_Q runs), one 1 KB get (REPS runs), and the
 *            ticks thread 0 waited for every streamer to start (all ones:
 *            they did not, and nothing was timed)
 *   Words a group never wrote stay 0xEEEEEEEE.
 *   Group k's create, initialize, start and join return codes:
 *   failed step 10k + 1 .. 10k + 4, the first one only.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_LEN 384
#define GROUPS 6
#define DATA_BYTES 0x10000
#define SOURCE_BYTES 0x4000
#define CTRL_AT (DATA_BYTES + 6 * SOURCE_BYTES)

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_result(const void *r, unsigned int len)
{
    unsigned int written;
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

static unsigned int result[RESULT_LEN / 4] __attribute__((aligned(128)));
static unsigned char data[CTRL_AT + 128] __attribute__((aligned(128)));

static void failed(unsigned int step, int rc)
{
    if (result[0] == 0) {
        result[0] = step;
        result[1] = (unsigned int)rc;
    }
}

/* One group of streamers + 1 threads; 0, or the step that failed. */
static unsigned int run_group(sysSpuImage *image, unsigned int streamers)
{
    int ret;
    unsigned int i;
    unsigned int cause = 0, status = 0;
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    sysSpuThreadGroupAttribute grpattr;
    sysSpuThreadAttribute thrattr;
    sysSpuThreadArgument thrargs;
    unsigned int base = 10 * streamers;

    memset(data + CTRL_AT, 0, 128);

    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 8;
    grpattr.name = "dmacont";
    ret = sysSpuThreadGroupCreate(&group, streamers + 1, 100, &grpattr);
    if (ret != 0) {
        failed(base + 1, ret);
        return base + 1;
    }
    for (i = 0; i <= streamers; i++) {
        memset(&thrattr, 0, sizeof(thrattr));
        thrattr.nsize = 8;
        thrattr.name = "dmacont";
        memset(&thrargs, 0, sizeof(thrargs));
        thrargs.arg0 = ((u64)i << 32) | streamers;
        thrargs.arg1 = (u64)(unsigned long)result;
        thrargs.arg2 = (u64)(unsigned long)data;
        ret = sysSpuThreadInitialize(&thread, group, i, image, &thrattr, &thrargs);
        if (ret != 0) {
            failed(base + 2, ret);
            sysSpuThreadGroupDestroy(group);
            return base + 2;
        }
    }
    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) {
        failed(base + 3, ret);
        sysSpuThreadGroupDestroy(group);
        return base + 3;
    }
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) {
        failed(base + 4, ret);
        sysSpuThreadGroupDestroy(group);
        return base + 4;
    }
    result[2 + 2 * streamers] = cause;
    result[3 + 2 * streamers] = status;
    sysSpuThreadGroupDestroy(group);
    return 0;
}

int main(void)
{
    int ret;
    unsigned int k;
    sysSpuImage image;

    for (k = 0; k < RESULT_LEN / 4; k++)
        result[k] = 0xEEEEEEEEu;
    result[0] = 0;
    result[1] = 0;
    result[14] = 0;
    result[15] = 0;
    memset(data, 0x33, sizeof(data));

    ret = CGOV_SPU_IMAGE_OPEN(&image, SPU_ELF_PATH);
    if (ret != 0) {
        failed(100, ret);
        write_result(result, RESULT_LEN);
        return 0;
    }
    for (k = 0; k < GROUPS; k++)
        (void)run_group(&image, k);
    sysSpuImageClose(&image);
    write_result(result, RESULT_LEN);
    return 0;
}
