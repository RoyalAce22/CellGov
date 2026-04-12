/* PPU program: barrier/wakeup ordering.
 *
 * Creates a thread group with two SPU threads sharing the same binary.
 * SPU 0 writes a flag to shared memory; SPU 1 polls until it sees the
 * flag, then writes its own result.  The PPU verifies that both
 * threads completed and that SPU 1 observed SPU 0's flag.
 *
 * The thread index is encoded in the low byte of arg1 (the buffer is
 * 256-byte aligned, so low 8 bits are available).
 *
 * Buffer layout (256 bytes):
 *   +0:   u32 spu0_status  (0 = pass)
 *   +4:   u32 spu0_marker  (0xAAAA0000)
 *   +8:   padding (8 bytes)
 *   +16:  u32 spu1_status  (0 = pass)
 *   +20:  u32 spu1_marker  (0xBBBB0001)
 *   +24:  padding (8 bytes)
 *   +128: u8[16] flag area  (inter-SPU signaling)
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

/* 256-byte aligned buffer for results + flag area. */
static unsigned char buf[256] __attribute__((aligned(256)));

int main(void)
{
    int ret;
    sysSpuImage image;
    sys_spu_group_t group;
    sys_spu_thread_t threads[2];
    sysSpuThreadGroupAttribute grpattr;
    sysSpuThreadAttribute thrattr;
    sysSpuThreadArgument thrargs;
    unsigned int cause, status;
    u64 base_ea = (u64)(unsigned long)buf;

    /* Zero the entire buffer (flag area starts at 0). */
    memset(buf, 0x00, sizeof(buf));
    /* Poison result slots so missing writes are detectable. */
    memset(buf, 0xFF, 32);

    ret = sysSpuImageOpen(&image, SPU_ELF_PATH);
    if (ret != 0) return fail(1);

    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 8;
    grpattr.name = "bar_grp";
    ret = sysSpuThreadGroupCreate(&group, 2, 100, &grpattr);
    if (ret != 0) return fail(2);

    /* Thread 0: index encoded in low byte of EA. */
    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 8;
    thrattr.name = "bar_sp0";
    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = base_ea | 0;
    ret = sysSpuThreadInitialize(&threads[0], group, 0, &image,
                                 &thrattr, &thrargs);
    if (ret != 0) return fail(3);

    /* Thread 1: index encoded in low byte of EA. */
    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 8;
    thrattr.name = "bar_sp1";
    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = base_ea | 1;
    ret = sysSpuThreadInitialize(&threads[1], group, 1, &image,
                                 &thrattr, &thrargs);
    if (ret != 0) return fail(4);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0) return fail(5);

    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0) return fail(6);

    /* Output both result slots (32 bytes total). */
    write_tty_tagged(buf, 32);

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);
    return 0;
}
