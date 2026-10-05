/* PPU probe: what `sc` answers for an r11 at and past the end of the
 * syscall dispatch table.
 *
 * Each case is one raw `sc` with every argument register zero. Slot 0
 * and slot 1023 are not-implemented stubs on 4.93; every other case is
 * at or past the table's 1024 slots. The last case sets bit 32 of r11
 * over 1: a kernel that compared only the low word would run slot 1,
 * sys_process_getpid, and answer the process id.
 *
 * Cases, in payload order (r11):
 *   0, 1023, 1024, 1025, 0xFFFF, 0x10000, 0x7FFFF, 0x80000,
 *   0x100000001
 *
 * Payload layout (72 bytes, big-endian): per case, u64 r3 after the sc.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define CASES 9

static const u64 R11[CASES] = {
    0, 1023, 1024, 1025, 0xFFFF, 0x10000, 0x7FFFF, 0x80000, 0x100000001ULL,
};

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

static volatile u64 result[CASES] __attribute__((aligned(128)));

static void write_frame(const volatile void *data, unsigned int len)
{
    unsigned int written;
    unsigned char len_be[4];
    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, len_be, 4, &written);
    sysTtyWrite(0, (const void *)data, len, &written);
    CGOV_OUT_FILE_WRITE((const void *)data, len);
}

int main(void)
{
    unsigned int i;

    for (i = 0; i < CASES; i++)
        result[i] = (u64)cgov_sc(R11[i], 0, 0, 0, 0, 0, 0, 0, 0);

    write_frame(result, sizeof(result));
    return 0;
}
