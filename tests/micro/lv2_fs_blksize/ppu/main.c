/* PPU program: the block size sys_fs_stat (808) answers on each device
 * a title reads from. Read-only: the probe stats and nothing else.
 *
 * Payload: 8 records of 4 big-endian u64 words, one per path:
 *   r3, st_mode, st_size, st_blksize
 * (the three stat words are SENTINEL when the stat refused).
 *
 *   0  /dev_hdd0/game/CGOV00001/USRDIR/EBOOT.BIN   the probe's own file
 *   1  /app_home/USRDIR/EBOOT.BIN                    the same, via app_home
 *   2  /app_home
 *   3  /dev_flash/sys/external/liblv2.sprx
 *   4  /dev_flash
 *   5  /dev_bdvd/PS3_GAME/PARAM.SFO                  only with a disc in
 *   6  /dev_bdvd
 *   7  /dev_usb000
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_FS_STAT 808

#define PATHS 8
#define WORDS (PATHS * 4)
#define SENTINEL 0xEEEEEEEEEEEEEEEEULL

static const char *const PATH[PATHS] = {
    "/dev_hdd0/game/CGOV00001/USRDIR/EBOOT.BIN",
    "/app_home/USRDIR/EBOOT.BIN",
    "/app_home",
    "/dev_flash/sys/external/liblv2.sprx",
    "/dev_flash",
    "/dev_bdvd/PS3_GAME/PARAM.SFO",
    "/dev_bdvd",
    "/dev_usb000",
};

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static volatile unsigned char stat_buf[64] __attribute__((aligned(16)));

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

static u64 be32(volatile unsigned char *p)
{
    return ((u64)p[0] << 24) | ((u64)p[1] << 16) | ((u64)p[2] << 8) | p[3];
}

static u64 be64(volatile unsigned char *p)
{
    return (be32(p) << 32) | be32(p + 4);
}

int main(void)
{
    unsigned i;

    for (i = 0; i < PATHS; i++) {
        u64 rc;
        memset((void *)stat_buf, 0xEE, sizeof(stat_buf));
        rc = (u64)cgov_sc(SYS_FS_STAT, EA(PATH[i]), EA(stat_buf), 0, 0, 0, 0, 0, 0);
        result[i * 4] = rc;
        if (rc == 0) {
            result[i * 4 + 1] = be32(stat_buf);
            result[i * 4 + 2] = be64(stat_buf + 40);
            result[i * 4 + 3] = be64(stat_buf + 48);
        } else {
            result[i * 4 + 1] = SENTINEL;
            result[i * 4 + 2] = SENTINEL;
            result[i * 4 + 3] = SENTINEL;
        }
    }
    write_frame();
    return 0;
}
