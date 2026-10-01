/* PPU program: DMA list get with a stall-and-notify element.
 *
 * Launches an SPU thread that gathers three 16-byte chunks with one list
 * get, stalling after the second and rewriting the third element before
 * it acknowledges the stall. After join, the PPU outputs the result.
 *
 * Result layout (64 bytes):
 *   +0:  u32 status (0), u32 stall-status word, 8 bytes padding
 *   +16: u8[48] gathered bytes
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"

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
    CGOV_OUT_FILE_WRITE(data, len);
}

static int __attribute__((noinline)) fail(unsigned int status)
{
    unsigned int buf[2] = { status, 0 };
    write_tty_tagged(buf, 8);
    return (int)status;
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

/* 256-byte aligned: 16-byte header, 48 gathered bytes, 64 source bytes. */
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

    memset(result_buf, 0xFF, sizeof(result_buf));

    ret = sysSpuImageOpen(&image, SPU_ELF_PATH);
    if (ret != 0) return fail(1);

    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 8;
    grpattr.name = "dmalist";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
    if (ret != 0) return fail(2);

    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 8;
    thrattr.name = "dmalist";
    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = (u64)(unsigned long)result_buf;
    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0) return fail(3);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(4);

    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(5);

    write_tty_tagged(result_buf, 16 + 48);

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);
    return 0;
}
