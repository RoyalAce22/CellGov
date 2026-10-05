/* PPU probe: which refusal sys_memory_allocate gives a size that is both
 * no page multiple and larger than the process's memory.
 *
 * Every case is one raw `sc 348`; the payload holds its return code, the
 * full r3 as the kernel left it. No case can succeed: each size is above
 * the user memory a game process has. The probe makes no object and
 * touches no file.
 *
 * Cases, in payload order (u64 r3 each, 8 bytes):
 *    0  0x10000001 bytes of 64 KiB pages  (no page multiple, too large)
 *    1  0x10000001 bytes of 1 MiB pages   (no page multiple, too large)
 *    2  0x10000000 bytes of 64 KiB pages  (a page multiple, too large)
 *
 * Payload: 3 u64 words, 24 bytes.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_MEMORY_ALLOCATE 348

#define PAGE_64K 0x200
#define PAGE_1M  0x400

#define WORDS 3

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static volatile unsigned int addr_word __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((unsigned long)(const void *)(p))

static void write_frame(void)
{
    unsigned int written;
    unsigned int len = sizeof(result);
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)result, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)result, len);
}

static u64 allocate(u64 size, u64 flags)
{
    return (u64)cgov_sc(SYS_MEMORY_ALLOCATE, size, flags, EA(&addr_word), 0, 0, 0, 0, 0);
}

int main(void)
{
    result[0] = allocate(0x10000001, PAGE_64K);
    result[1] = allocate(0x10000001, PAGE_1M);
    result[2] = allocate(0x10000000, PAGE_64K);
    write_frame();
    return 0;
}
