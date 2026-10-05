/* PPU probe: the CellError codes the file-system calls give under
 * /dev_hdd0/tmp, one call apiece.
 *
 * Every case is one raw `sc`; the payload holds its return code, the
 * full r3 as the kernel left it. The probe makes one directory and one
 * file under /dev_hdd0/tmp, removes both, and touches no other path.
 * The directory is created with mode 0700 and the file with 0600, the
 * modes the file system accepts for each.
 *
 * Cases, in payload order (u64 r3 each, 8 bytes):
 *    0  sc 801 open of a missing file                      (ENOENT)
 *    1  sc 811 mkdir of the test directory again           (EEXIST)
 *    2  sc 801 open of the test directory as a file        (EISDIR)
 *    3  sc 801 create of the test file again, O_EXCL       (EEXIST)
 *    4  sc 805 opendir of the test file                    (ENOTDIR)
 *    5  sc 813 rmdir of the non-empty test directory       (ENOTEMPTY)
 *    6  sc 804 close of fd 0x7FFF                          (EBADF)
 *    7  sc 801 open of a path with a 300-byte name         (ENAMETOOLONG)
 *
 * Setup and cleanup answers, after the cases (u64 r3 each):
 *    8  mkdir             9  file create     10  file close
 *   11  unlink           12  rmdir
 *
 * Payload: 13 u64 words, 104 bytes.
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
#define SYS_FS_RMDIR    813
#define SYS_FS_UNLINK   814

#define O_RDONLY 0x0
#define O_WRONLY 0x1
#define O_CREAT  0x40
#define O_EXCL   0x80
#define MODE_FILE 0x180
#define MODE_DIR  0x1C0

#define WORDS 13

static const char TEST_DIR[] = "/dev_hdd0/tmp/cgov_errfs_dir";
static const char TEST_FILE[] = "/dev_hdd0/tmp/cgov_errfs_dir/file";
static const char MISSING[] = "/dev_hdd0/tmp/cgov_errfs_missing";

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static volatile int fd_word __attribute__((aligned(8)));
static char long_path[400];

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

    for (i = 0; i < WORDS; i++)
        result[i] = 0xEEEEEEEEEEEEEEEEULL;

    /* A directory or file left by an earlier run goes first, so the
     * creates below start from nothing. */
    (void)sc(SYS_FS_UNLINK, EA(TEST_FILE), 0, 0, 0);
    (void)sc(SYS_FS_RMDIR, EA(TEST_DIR), 0, 0, 0);

    result[0] = fs_open(MISSING, O_RDONLY, 0);
    if (result[0] == 0)
        (void)sc(SYS_FS_CLOSE, (u64)fd_word, 0, 0, 0);
    result[8] = sc(SYS_FS_MKDIR, EA(TEST_DIR), MODE_DIR, 0, 0);
    result[1] = sc(SYS_FS_MKDIR, EA(TEST_DIR), MODE_DIR, 0, 0);
    result[2] = fs_open(TEST_DIR, O_RDONLY, 0);
    if (result[2] == 0)
        (void)sc(SYS_FS_CLOSE, (u64)fd_word, 0, 0, 0);
    result[9] = fs_open(TEST_FILE, O_WRONLY | O_CREAT | O_EXCL, MODE_FILE);
    if (result[9] == 0)
        result[10] = sc(SYS_FS_CLOSE, (u64)fd_word, 0, 0, 0);
    result[3] = fs_open(TEST_FILE, O_WRONLY | O_CREAT | O_EXCL, MODE_FILE);
    if (result[3] == 0)
        (void)sc(SYS_FS_CLOSE, (u64)fd_word, 0, 0, 0);
    fd_word = -1;
    result[4] = sc(SYS_FS_OPENDIR, EA(TEST_FILE), EA(&fd_word), 0, 0);
    if (result[4] == 0)
        (void)sc(SYS_FS_CLOSEDIR, (u64)fd_word, 0, 0, 0);
    result[5] = sc(SYS_FS_RMDIR, EA(TEST_DIR), 0, 0, 0);
    result[6] = sc(SYS_FS_CLOSE, 0x7FFF, 0, 0, 0);
    memset(long_path, 0, sizeof(long_path));
    memcpy(long_path, "/dev_hdd0/tmp/", 14);
    for (i = 14; i < 14 + 300; i++)
        long_path[i] = 'n';
    result[7] = fs_open(long_path, O_RDONLY, 0);
    if (result[7] == 0)
        (void)sc(SYS_FS_CLOSE, (u64)fd_word, 0, 0, 0);
    result[11] = sc(SYS_FS_UNLINK, EA(TEST_FILE), 0, 0, 0);
    result[12] = sc(SYS_FS_RMDIR, EA(TEST_DIR), 0, 0, 0);

    write_frame();
    return 0;
}
