/* PPU program: run the SPU program as the one thread of an SPU thread
 * group and read back whether the transfers it queued before it exited
 * the group are in the buffer when the join returns.
 *
 * 1. Zero the 256 KB buffer and open the SPU image.
 * 2. Create a group of one thread, initialize the thread with the
 *    buffer's effective address as its first argument, and start the
 *    group.
 * 3. Join the group, and at once read the first and the last word of
 *    each 16 KB of the buffer.
 * 4. Spin 1000000 times, then read the same words again.
 * 5. Destroy the group, close the image and write one CGOV frame.
 *
 * Payload layout (272 bytes, big-endian words):
 *   +0   step that failed (0 = none)
 *   +4   that step's return code
 *   +8   join cause            +12 join status
 *   +16  first word of each 16 KB when the join returns (16 words)
 *   +80  last word of each 16 KB when the join returns (16 words)
 *   +144 first word of each 16 KB later (16 words)
 *   +208 last word of each 16 KB later (16 words)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define CHUNK 0x4000u
#define CHUNKS 16
#define SPINS 1000000u

struct Result {
    unsigned int failed_step;
    unsigned int failed_rc;
    unsigned int cause;
    unsigned int status;
    unsigned int first_at_join[CHUNKS];
    unsigned int last_at_join[CHUNKS];
    unsigned int first_later[CHUNKS];
    unsigned int last_later[CHUNKS];
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

static void read_words(unsigned int *first, unsigned int *last)
{
    unsigned int i;
    for (i = 0; i < CHUNKS; i++) {
        first[i] = buffer[i * CHUNK / 4];
        last[i] = buffer[(i + 1) * CHUNK / 4 - 1];
    }
}

int main(void)
{
    int ret;
    sysSpuImage image;
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    sysSpuThreadGroupAttribute grpattr;
    sysSpuThreadAttribute thrattr;
    sysSpuThreadArgument thrargs;
    unsigned int cause = 0, status = 0;
    volatile unsigned int spin;
    unsigned int i;

    memset(&result, 0, sizeof(result));
    for (i = 0; i < CHUNKS * CHUNK / 4; i++)
        buffer[i] = 0;

    ret = CGOV_SPU_IMAGE_OPEN(&image, SPU_ELF_PATH);
    if (ret != 0)
        return fail(1, ret);

    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 8;
    grpattr.name = "dma_grp";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
    if (ret != 0) {
        sysSpuImageClose(&image);
        return fail(2, ret);
    }

    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 8;
    thrattr.name = "dma_spu";
    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg0 = (u64)(unsigned long)buffer;
    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0) {
        sysSpuThreadGroupDestroy(group);
        sysSpuImageClose(&image);
        return fail(3, ret);
    }

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) {
        sysSpuThreadGroupDestroy(group);
        sysSpuImageClose(&image);
        return fail(4, ret);
    }

    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    read_words(result.first_at_join, result.last_at_join);
    if (ret != 0) {
        sysSpuThreadGroupDestroy(group);
        sysSpuImageClose(&image);
        return fail(5, ret);
    }
    result.cause = cause;
    result.status = status;

    for (spin = 0; spin < SPINS; spin++)
        ;
    read_words(result.first_later, result.last_later);

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);
    write_result(&result);
    return 0;
}
