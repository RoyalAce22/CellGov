/* PPU program: host the spu_events SPU program and act on the phases
 * that need another processor.
 *
 * The SPU raises a flag word in the first line of a shared region for
 * each phase the PPU must act on:
 *   6  write the SPU's inbound mailbox
 *   7  write its signal notification 1 register
 *   8  store to the line the SPU reserved (the region's second line)
 *   11 read the time base: the SPU has just read the decrementer
 *   12 read the time base again: the SPU read it after its spin
 *
 * Result frame (224 bytes, big-endian): the SPU's 192-byte phase block,
 * then
 *   +192 u64 time base at flag 11
 *   +200 u64 time base at flag 12
 *   +208 u64 time-base frequency
 *   +216 u32 the first phase whose flag never came (0 = none)
 *   +220 u32 the failed step and its code in the low byte (0 = none)
 */

#include <string.h>

#include <sys/process.h>
#include <sys/spu.h>
#include <sys/systime.h>
#include <lv2/spu.h>
#include <sys/tty.h>

#include "cgov_out.h"
#include "cgov_spu_load.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define RESULT_BYTES 224
/* Seconds the PPU waits for a flag before it gives up on the phase. */
#define FLAG_SECONDS 20

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static void write_frame(const void *payload, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, payload, len, &written);
    CGOV_OUT_FILE_WRITE(payload, len);
}

static unsigned char results[RESULT_BYTES] __attribute__((aligned(128)));
/* The flag in the first line, the scratch area from the second. */
static unsigned char region[128 + 4096] __attribute__((aligned(128)));

static void put_u64(unsigned int at, unsigned long long value)
{
    int i;
    for (i = 0; i < 8; i++)
        results[at + i] = (unsigned char)(value >> (56 - 8 * i));
}

static void put_u32(unsigned int at, unsigned int value)
{
    int i;
    for (i = 0; i < 4; i++)
        results[at + i] = (unsigned char)(value >> (24 - 8 * i));
}

static unsigned long long timebase(void)
{
    unsigned long long tb;
    __asm__ volatile ("mftb %0" : "=r"(tb));
    return tb;
}

static int __attribute__((noinline)) fail(unsigned int step, int rc)
{
    put_u32(220, (step << 8) | ((unsigned int)rc & 0xFF));
    write_frame(results, RESULT_BYTES);
    return (int)step;
}

/* Wait for the SPU to raise `phase`, at most FLAG_SECONDS of time base;
 * return 0 when it came. */
static int await_flag(unsigned int phase)
{
    volatile unsigned int *flag = (volatile unsigned int *)region;
    unsigned long long deadline =
        timebase() + FLAG_SECONDS * sysGetTimebaseFrequency();
    while (*flag != phase && timebase() < deadline)
        ;
    return *flag == phase ? 0 : 1;
}

static const char SPU_ELF_PATH[] = "/app_home/spu_main.elf";

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
    unsigned int missed = 0;
    unsigned long long tb_11 = 0, tb_12 = 0;
    volatile unsigned int *reserved = (volatile unsigned int *)(region + 128);

    memset(results, 0xEE, sizeof(results));
    memset(region, 0, sizeof(region));

    ret = CGOV_SPU_IMAGE_OPEN(&image, SPU_ELF_PATH);
    if (ret != 0)
        return fail(1, ret);

    memset(&grpattr, 0, sizeof(grpattr));
    grpattr.nsize = 9;
    grpattr.name = "test_grp";
    ret = sysSpuThreadGroupCreate(&group, 1, 100, &grpattr);
    if (ret != 0)
        return fail(2, ret);

    memset(&thrattr, 0, sizeof(thrattr));
    thrattr.nsize = 9;
    thrattr.name = "test_spu";
    memset(&thrargs, 0, sizeof(thrargs));
    thrargs.arg1 = (u64)(unsigned long)results;
    thrargs.arg2 = (u64)(unsigned long)region;
    ret = sysSpuThreadInitialize(&thread, group, 0, &image, &thrattr, &thrargs);
    if (ret != 0)
        return fail(3, ret);

    ret = sysSpuThreadGroupStart(group);
    if (ret != 0)
        return fail(4, ret);

    if (await_flag(6) == 0)
        sysSpuThreadWriteMb(thread, 0xABCD1234u);
    else if (missed == 0)
        missed = 6;
    if (await_flag(7) == 0)
        sysSpuThreadWriteSignal(thread, 0, 0x51515151u);
    else if (missed == 0)
        missed = 7;
    if (await_flag(8) == 0) {
        *reserved = 0xDEADBEEFu;
        __asm__ volatile ("sync" ::: "memory");
    } else if (missed == 0) {
        missed = 8;
    }
    if (await_flag(11) == 0)
        tb_11 = timebase();
    else if (missed == 0)
        missed = 11;
    if (await_flag(12) == 0)
        tb_12 = timebase();
    else if (missed == 0)
        missed = 12;

    ret = sysSpuThreadGroupJoin(group, &cause, &status);
    if (ret != 0)
        return fail(5, ret);

    put_u64(192, tb_11);
    put_u64(200, tb_12);
    put_u64(208, sysGetTimebaseFrequency());
    put_u32(216, missed);
    put_u32(220, 0);
    write_frame(results, RESULT_BYTES);

    sysSpuThreadGroupDestroy(group);
    sysSpuImageClose(&image);
    return 0;
}
