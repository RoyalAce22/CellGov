/* PPU program: proof-of-life for managed SPU thread groups.
 *
 * 1. Load the SPU ELF from the virtual filesystem.
 * 2. Create a thread group with one thread.
 * 3. Start the group.
 * 4. Join (wait for all threads to exit).
 * 5. Read the fixed value from the SPU's local store.
 * 6. Write the tagged result to TTY for harness extraction.
 * 7. Exit.
 *
 * The SPU program writes 0x1337BAAD at LS 0x100 and exits.
 * The PPU reads LS 0x100 after joining and outputs the result
 * using the CGOV TTY protocol (4-byte magic + 4-byte length +
 * payload).
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

/* No printf/exit -- the custom CRT0 does not initialize the full
 * newlib runtime. Error paths write status to TTY and return
 * nonzero from main; CRT0 calls sys_process_exit with the
 * return value. */

SYS_PROCESS_PARAM(1001, 0x10000)

/* Two-word fixed result layout shared across the microtest corpus:
 * status is 0 on pass, value carries the test-specific result word. */
struct TestResult {
    unsigned int status;
    unsigned int value;
};

/* TTY protocol tag. */
static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_tty_result(const struct TestResult *r);

/* Report a failure status code and return it. Noinline prevents the
 * compiler from merging error paths with rldicr instructions. */
static int __attribute__((noinline)) fail(unsigned int status)
{
    struct TestResult r;
    r.status = status;
    r.value  = 0;
    write_tty_result(&r);
    return (int)status;
}

static void write_tty_result(const struct TestResult *r)
{
    unsigned int len;
    unsigned int written;

    len = sizeof(*r);

    /* Convert length to big-endian bytes. PS3 is big-endian natively. */
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;

    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, r, len, &written);
}

/* SPU ELF path on the PS3 virtual filesystem. RPCS3 maps /app_home/
 * to the directory containing the launched PPU ELF. */
static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

/* Result buffer in main memory. 128-byte aligned for DMA safety. */
static struct TestResult result __attribute__((aligned(128)));

int main(void)
{
    int ret;
    sysSpuImage image;
    sys_spu_group_t group;
    sys_spu_thread_t thread;
    sysSpuThreadGroupAttribute grpattr;
    sysSpuThreadAttribute thrattr;
    sysSpuThreadArgument thrargs;
    unsigned int cause, status;

    /* Poison the buffer so partial DMA is detectable. */
    result.status = 0xFFFFFFFF;
    result.value  = 0xFFFFFFFF;

    /* Step 1: load SPU ELF from the filesystem. */
    ret = sysSpuImageOpen(&image, SPU_ELF_PATH);
    if (ret != 0)
        return fail(1);

    /* Step 2: create thread group. */
    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 9;
    grpattr.name = "test_grp";

    ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
    if (ret != 0)
        return fail(2);

    /* Step 3: create SPU thread. Pass result buffer EA as arg1. */
    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 9;
    thrattr.name = "test_spu";

    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = (u64)(unsigned long)&result;

    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0)
        return fail(3);

    /* Step 4: start the thread group. */
    ret = sysSpuThreadGroupStart(group);
    if (ret != 0)
        return fail(4);

    /* Step 5: join -- blocks until all threads exit. */
    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0)
        return fail(5);

    /* The SPU DMA'd the TestResult to &result before exiting. */
    write_tty_result(&result);

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);

    return 0;
}
