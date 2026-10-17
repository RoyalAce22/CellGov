/* PPU probe: the CellError codes the file-system calls give for an
 * unmounted device, a cross-device rename, a mode change and the
 * per-process open-file limit.
 *
 * Every call is one raw `sc`; the payload holds the full r3 the kernel
 * left. The probe makes one directory (mode 0700) and one file (mode
 * 0600) under /dev_hdd0/tmp, and removes both. It writes nowhere else:
 * a rename that lands on /dev_hdd1 is unlinked there.
 *
 * Cases, in payload order (u64 each, 8 bytes):
 *    0  sc 801 open /dev_usb007/cgov_x, read-only
 *    1  sc 805 opendir /dev_usb007
 *    2  sc 801 open /dev_bdvd/cgov_x, read-only (no disc inserted)
 *    3  sc 834 chmod of the test file to 0400
 *    4  sc 801 open of the test file write-only, after that chmod
 *    5  sc 834 chmod of the test file back to 0600
 *    6  opens of the test file, read-only, that succeeded before the
 *       first refusal (at most OPEN_CAP)
 *    7  the r3 of that first refusal (0 when none came)
 *    8  the fd of the first successful open
 *    9  the fd of the last successful open
 *   10  closes of those fds that did not return 0
 *   11  sc 812 rename of the test file to /dev_usb007/cgov_x
 *   12  sc 812 rename of the test file to /dev_hdd1/cgov_errfs_more
 *
 * Setup and cleanup answers (u64 each):
 *   13  mkdir   14  file create   15  file close
 *   16  unlink of /dev_hdd1/cgov_errfs_more (ENOENT unless case 12 moved it)
 *   17  unlink of the test file   18  rmdir
 *
 * Payload: 19 u64 words, 152 bytes.
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
#define SYS_FS_MKDIR    811
#define SYS_FS_RENAME   812
#define SYS_FS_RMDIR    813
#define SYS_FS_UNLINK   814
#define SYS_FS_CHMOD    834

#define O_RDONLY 0x0
#define O_WRONLY 0x1
#define O_CREAT  0x40
#define O_EXCL   0x80
#define MODE_FILE      0x180
#define MODE_FILE_READ 0x100
#define MODE_DIR       0x1C0

#define OPEN_CAP 300

#define WORDS 19

static const char TEST_DIR[] = "/dev_hdd0/tmp/cgov_errfs_more";
static const char TEST_FILE[] = "/dev_hdd0/tmp/cgov_errfs_more/file";
static const char USB_FILE[] = "/dev_usb007/cgov_x";
static const char USB_DIR[] = "/dev_usb007";
static const char BDVD_FILE[] = "/dev_bdvd/cgov_x";
static const char HDD1_FILE[] = "/dev_hdd1/cgov_errfs_more";

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static volatile int fd_word __attribute__((aligned(8)));
static int fds[OPEN_CAP];

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

static u64 sc(u64 num, u64 a, u64 b, u64 c, u64 d)
{
    return (u64)cgov_sc(num, a, b, c, d, 0, 0, 0, 0);
}

static u64 fs_open(const char *path, u64 flags, u64 mode)
{
    fd_word = -1;
    return (u64)cgov_sc(SYS_FS_OPEN, EA(path), flags, EA(&fd_word), mode, 0, 0, 0, 0);
}

int main(void)
{
    unsigned int i;
    unsigned int opened = 0;
    u64 rc;

    for (i = 0; i < WORDS; i++)
        result[i] = 0xEEEEEEEEEEEEEEEEULL;

    /* Anything an earlier run left goes first. */
    (void)sc(SYS_FS_UNLINK, EA(HDD1_FILE), 0, 0, 0);
    (void)sc(SYS_FS_UNLINK, EA(TEST_FILE), 0, 0, 0);
    (void)sc(SYS_FS_RMDIR, EA(TEST_DIR), 0, 0, 0);

    result[13] = sc(SYS_FS_MKDIR, EA(TEST_DIR), MODE_DIR, 0, 0);
    result[14] = fs_open(TEST_FILE, O_WRONLY | O_CREAT | O_EXCL, MODE_FILE);
    if (result[14] == 0)
        result[15] = sc(SYS_FS_CLOSE, (u64)fd_word, 0, 0, 0);

    result[0] = fs_open(USB_FILE, O_RDONLY, 0);
    if (result[0] == 0)
        (void)sc(SYS_FS_CLOSE, (u64)fd_word, 0, 0, 0);
    fd_word = -1;
    result[1] = sc(SYS_FS_OPENDIR, EA(USB_DIR), EA(&fd_word), 0, 0);
    if (result[1] == 0)
        (void)sc(SYS_FS_CLOSEDIR, (u64)fd_word, 0, 0, 0);
    result[2] = fs_open(BDVD_FILE, O_RDONLY, 0);
    if (result[2] == 0)
        (void)sc(SYS_FS_CLOSE, (u64)fd_word, 0, 0, 0);

    result[3] = sc(SYS_FS_CHMOD, EA(TEST_FILE), MODE_FILE_READ, 0, 0);
    result[4] = fs_open(TEST_FILE, O_WRONLY, 0);
    if (result[4] == 0)
        (void)sc(SYS_FS_CLOSE, (u64)fd_word, 0, 0, 0);
    result[5] = sc(SYS_FS_CHMOD, EA(TEST_FILE), MODE_FILE, 0, 0);

    result[7] = 0;
    while (opened < OPEN_CAP) {
        rc = fs_open(TEST_FILE, O_RDONLY, 0);
        if (rc != 0) {
            result[7] = rc;
            break;
        }
        fds[opened++] = fd_word;
    }
    result[6] = opened;
    result[8] = opened ? (u64)fds[0] : 0;
    result[9] = opened ? (u64)fds[opened - 1] : 0;
    result[10] = 0;
    for (i = 0; i < opened; i++)
        if (sc(SYS_FS_CLOSE, (u64)fds[i], 0, 0, 0) != 0)
            result[10]++;

    result[11] = sc(SYS_FS_RENAME, EA(TEST_FILE), EA(USB_FILE), 0, 0);
    result[12] = sc(SYS_FS_RENAME, EA(TEST_FILE), EA(HDD1_FILE), 0, 0);

    result[16] = sc(SYS_FS_UNLINK, EA(HDD1_FILE), 0, 0, 0);
    result[17] = sc(SYS_FS_UNLINK, EA(TEST_FILE), 0, 0, 0);
    result[18] = sc(SYS_FS_RMDIR, EA(TEST_DIR), 0, 0, 0);

    write_frame();
    return 0;
}
