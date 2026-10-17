/* PPU probe: where sys_fs_stat (808) stops taking a long path, byte by
 * byte, under two mount points of different length. Every path names
 * nothing, so a path the kernel takes answers ENOENT; it starts with one
 * of the BASES and goes on in components of 'a' no longer than 200
 * bytes. The probe creates nothing.
 *
 * Payload: big-endian u64 words: for each base in BASES, for each length
 * from FIRST to FIRST + 7, 808's return code for a path of that many
 * bytes before the NUL.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_FS_STAT 808
#define FIRST 1033
#define NLENS 8

static const char *const BASES[2] = {
    "/dev_hdd0/tmp/cgov_fspl_missing",
    "/dev_flash/cgov_fspl_missing",
};

static volatile u64 result[2 * NLENS] __attribute__((aligned(128)));
static volatile unsigned char stat_buf[64] __attribute__((aligned(16)));
static char path[1200] __attribute__((aligned(16)));

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

/* `path` = `base`, then components of 'a' (none longer than 200 bytes)
 * making `len` bytes before the NUL. */
static void long_path(const char *base, unsigned len)
{
    unsigned at = strlen(base);
    memcpy(path, base, at);
    while (at < len) {
        unsigned run = 0;
        path[at++] = '/';
        while (at < len && run < 200) {
            path[at++] = 'a';
            run++;
        }
    }
    if (path[len - 1] == '/')
        path[len - 1] = 'a';
    path[len] = 0;
}

int main(void)
{
    unsigned b, i;
    for (b = 0; b < 2; b++) {
        for (i = 0; i < NLENS; i++) {
            long_path(BASES[b], FIRST + i);
            result[b * NLENS + i] =
                (u64)cgov_sc(SYS_FS_STAT, EA(path), EA(stat_buf), 0, 0, 0, 0, 0, 0);
        }
    }
    write_frame();
    return 0;
}
