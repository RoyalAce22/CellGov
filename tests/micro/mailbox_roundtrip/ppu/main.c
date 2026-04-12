/* PPU program: mailbox roundtrip.
 *
 * Sends command 0x42 to the SPU via inbound mailbox, waits for the
 * SPU to DMA its response to a shared buffer, outputs result via
 * CGOV TTY protocol.
 *
 * Expected: SPU returns ~0x42 = 0xFFFFFFBD.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

SYS_PROCESS_PARAM(1001, 0x10000)

struct TestResult {
    unsigned int status;
    unsigned int value;
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_tty_result(const struct TestResult *r);

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
    unsigned int len = sizeof(*r);
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, r, len, &written);
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";
static struct TestResult result __attribute__((aligned(128)));

#define COMMAND_VALUE 0x42

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

    result.status = 0xFFFFFFFF;
    result.value  = 0xFFFFFFFF;

    ret = sysSpuImageOpen(&image, SPU_ELF_PATH);
    if (ret != 0)
        return fail(1);

    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 9;
    grpattr.name = "mbox_grp";

    ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
    if (ret != 0)
        return fail(2);

    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 9;
    thrattr.name = "mbox_spu";

    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = (u64)(unsigned long)&result;

    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0)
        return fail(3);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0)
        return fail(4);

    /* Send the command to the SPU's inbound mailbox. */
    ret = sysSpuThreadWriteMb(thread, COMMAND_VALUE);
    if (ret != 0)
        return fail(5);

    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0)
        return fail(6);

    /* SPU DMA'd the result to &result. */
    write_tty_result(&result);

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);
    return 0;
}
