/* PPU probe: what sys_fs_open (801) and sys_fs_opendir (805) answer for
 * a path on each kind of root a game process can name: a device that is
 * mounted, a device that is not, a root no device has, and the
 * process-relative roots.
 *
 * Every call is one read-only raw `sc`; the payload holds the full r3
 * the kernel left. A call that opens closes its fd. Nothing is created.
 *
 * Cases, in payload order (u64 each, 8 bytes):
 *    0  open    /dev_hdd0/cgov_missing     (mounted device, missing name)
 *    1  open    /dev_usb000/cgov_missing   (mounted USB, missing name)
 *    2  open    /dev_usb007/cgov_x         (USB port with nothing in it)
 *    3  open    /dev_cgov/cgov_x           (a device name no driver has)
 *    4  open    /cgov_nodev/cgov_x         (a root that is not a device)
 *    5  open    /app_home/cgov_x
 *    6  open    /host_root/cgov_x
 *    7  open    cgov_relative              (no leading slash)
 *    8  open    /dev_hdd1/cgov_x
 *    9  opendir /cgov_nodev
 *   10  opendir /dev_cgov
 *   11  opendir /app_home
 *   12  open    /                          (the root itself)
 *   13  opendir /
 *
 * Payload: 14 u64 words, 112 bytes.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_FS_OPEN     801
#define SYS_FS_CLOSE    804
#define SYS_FS_OPENDIR  805
#define SYS_FS_CLOSEDIR 807

#define O_RDONLY 0x0

#define WORDS 14

static const char *const OPEN_PATHS[] = {
    "/dev_hdd0/cgov_missing",
    "/dev_usb000/cgov_missing",
    "/dev_usb007/cgov_x",
    "/dev_cgov/cgov_x",
    "/cgov_nodev/cgov_x",
    "/app_home/cgov_x",
    "/host_root/cgov_x",
    "cgov_relative",
    "/dev_hdd1/cgov_x",
};

static const char *const DIR_PATHS[] = {
    "/cgov_nodev",
    "/dev_cgov",
    "/app_home",
};

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static volatile int fd_word __attribute__((aligned(8)));

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

static u64 fs_open(const char *path)
{
    u64 rc;
    fd_word = -1;
    rc = (u64)cgov_sc(SYS_FS_OPEN, EA(path), O_RDONLY, EA(&fd_word), 0, 0, 0, 0, 0);
    if (rc == 0)
        (void)cgov_sc(SYS_FS_CLOSE, (u64)fd_word, 0, 0, 0, 0, 0, 0, 0);
    return rc;
}

static u64 fs_opendir(const char *path)
{
    u64 rc;
    fd_word = -1;
    rc = (u64)cgov_sc(SYS_FS_OPENDIR, EA(path), EA(&fd_word), 0, 0, 0, 0, 0, 0);
    if (rc == 0)
        (void)cgov_sc(SYS_FS_CLOSEDIR, (u64)fd_word, 0, 0, 0, 0, 0, 0, 0);
    return rc;
}

int main(void)
{
    unsigned int i;

    for (i = 0; i < WORDS; i++)
        result[i] = 0xEEEEEEEEEEEEEEEEULL;

    for (i = 0; i < 9; i++)
        result[i] = fs_open(OPEN_PATHS[i]);
    for (i = 0; i < 3; i++)
        result[9 + i] = fs_opendir(DIR_PATHS[i]);
    result[12] = fs_open("/");
    result[13] = fs_opendir("/");

    write_frame();
    return 0;
}
