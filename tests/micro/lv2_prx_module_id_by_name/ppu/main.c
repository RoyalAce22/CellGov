/* PPU probe: what _sys_prx_get_module_id_by_name (syscall 496) answers
 * for a loaded module, for names no loaded module carries, and for
 * pointers that name no readable string.
 *
 * Each case is one raw `sc` with r11 = 496, r3 = the name pointer,
 * r4 = 0 (flags) and r5 = 0 (no option structure).
 *
 * Cases, in payload order:
 *   0  "liblv2"          the module-info name of liblv2, loaded in
 *                        every process
 *   1  "cellSpursTuner"  a module that is not loaded
 *   2  "liblv"           a prefix of a loaded name
 *   3  "LIBLV2"          a loaded name in another case
 *   4  ""                the empty name
 *   5  NULL              no string
 *   6  "liblv2" with bit 32 of the pointer set
 *
 * Payload layout (56 bytes, big-endian): per case, u64 r3 after the sc.
 */

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define CASES 7
#define SYS_PRX_GET_MODULE_ID_BY_NAME 496

static const char LIBLV2[] = "liblv2";
static const char TUNER[] = "cellSpursTuner";
static const char PREFIX[] = "liblv";
static const char UPPER[] = "LIBLV2";
static const char EMPTY[] = "";

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

static u64 by_name(u64 name)
{
    return (u64)cgov_sc(SYS_PRX_GET_MODULE_ID_BY_NAME, name, 0, 0, 0, 0, 0, 0, 0);
}

int main(void)
{
    const u64 ptr[CASES] = {
        (u64)(unsigned long)LIBLV2,
        (u64)(unsigned long)TUNER,
        (u64)(unsigned long)PREFIX,
        (u64)(unsigned long)UPPER,
        (u64)(unsigned long)EMPTY,
        0,
        0x100000000ULL | (u64)(unsigned long)LIBLV2,
    };
    unsigned int i;

    for (i = 0; i < CASES; i++)
        result[i] = by_name(ptr[i]);

    write_frame(result, sizeof(result));
    return 0;
}
