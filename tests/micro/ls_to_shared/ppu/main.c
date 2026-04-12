/* PPU program: local-store to shared-memory publication.
 *
 * Launches an SPU thread that computes a 32-word chain in local store
 * (each word = previous + 0x01010101, starting from 0xC0DE0000) and
 * DMA puts the result to a shared buffer.  The PPU verifies the chain
 * after join.
 *
 * Result layout (256 bytes):
 *   +0:  u32 status (0 = pass)
 *   +4:  u32 count  (32)
 *   +8:  padding (8 bytes)
 *   +16: u32[32] computed data (128 bytes)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

SYS_PROCESS_PARAM(1001, 0x10000)

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_tty_tagged(const void *data, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, data, len, &written);
}

static int __attribute__((noinline)) fail(unsigned int status)
{
    unsigned int buf[2] = { status, 0 };
    write_tty_tagged(buf, 8);
    return (int)status;
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

static unsigned char result_buf[256] __attribute__((aligned(256)));

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

    /* Poison the buffer. */
    memset(result_buf, 0xFF, sizeof(result_buf));

    ret = sysSpuImageOpen(&image, SPU_ELF_PATH);
    if (ret != 0) return fail(1);

    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 8;
    grpattr.name = "ls_grp\0";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
    if (ret != 0) return fail(2);

    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 8;
    thrattr.name = "ls_spu\0";
    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = (u64)(unsigned long)result_buf;
    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0) return fail(3);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(4);

    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(5);

    /* Output status header (16 bytes) + computed data (128 bytes). */
    write_tty_tagged(result_buf, 16 + 128);

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);
    return 0;
}
