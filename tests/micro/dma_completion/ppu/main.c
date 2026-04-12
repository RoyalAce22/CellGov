/* PPU program: DMA put completion visibility.
 *
 * Launches an SPU thread that DMA puts a 128-byte repeating pattern
 * (0xDE 0xAD 0xBE 0xEF) to a shared buffer. After join, the PPU
 * verifies byte-level correctness and outputs the result.
 *
 * Result layout (144 bytes):
 *   +0:  u32 status (0=pass, nonzero=first bad offset+1)
 *   +4:  u32 pattern_size (128)
 *   +8:  padding (8 bytes)
 *   +16: u8[128] pattern data
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

/* 256-byte aligned buffer: 16 bytes header + 128 bytes pattern. */
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
    grpattr.name = "dma_grp";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
    if (ret != 0) return fail(2);

    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 8;
    thrattr.name = "dma_spu";
    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = (u64)(unsigned long)result_buf;
    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0) return fail(3);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(4);

    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(5);

    /* Output the full buffer (header + pattern) via CGOV. */
    write_tty_tagged(result_buf, 16 + 128);

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);
    return 0;
}
