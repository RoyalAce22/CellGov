/* PPU probe: which directories a process without root permission may
 * create a file in. The build signs this probe's EBOOT with
 * control_flags1 0, as a retail title is signed.
 *
 * Each case is one sc 801 create (O_WRONLY | O_CREAT | O_TRUNC, mode
 * 0600) of a file, closed and unlinked again when it opens:
 *    0  /dev_hdd0/tmp/cgov_lv2_fs_user_write.bin
 *    1  /dev_hdd0/game/CGOV00001/USRDIR/cgov_lv2_fs_user_write.bin
 *    2  /dev_hdd0/cgov_lv2_fs_user_write.bin
 *
 * The payload (3 u64 words, 24 bytes) holds each create's r3. It goes
 * to the TTY, and to the first of those files that opens: a capture
 * fetches it from /dev_hdd0/tmp, and a run that cannot write there
 * leaves it in USRDIR for an FTP read.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_FS_OPEN   801
#define SYS_FS_WRITE  803
#define SYS_FS_CLOSE  804
#define SYS_FS_UNLINK 814

#define O_WRONLY 0x1
#define O_CREAT  0x40
#define O_TRUNC  0x200
#define MODE_FILE 0x180

#define WORDS 3

static const char *const PATHS[WORDS] = {
    "/dev_hdd0/tmp/cgov_lv2_fs_user_write.bin",
    "/dev_hdd0/game/CGOV00001/USRDIR/cgov_lv2_fs_user_write.bin",
    "/dev_hdd0/cgov_lv2_fs_user_write.bin",
};

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static volatile int fd_word __attribute__((aligned(8)));
static volatile u64 nwritten __attribute__((aligned(8)));

static const char CGOV_MAGIC[4] = { 'C', 'G', 'O', 'V' };

#define EA(p) ((unsigned long)(const void *)(p))

static u64 create(const char *path)
{
    fd_word = -1;
    return (u64)cgov_sc(SYS_FS_OPEN, EA(path), O_WRONLY | O_CREAT | O_TRUNC, EA(&fd_word),
                        MODE_FILE, 0, 0, 0, 0);
}

int main(void)
{
    unsigned int i, written;
    unsigned int len = sizeof(result);
    unsigned char frame_head[8] = { 'C', 'G', 'O', 'V', 0, 0, 0, 0 };

    for (i = 0; i < WORDS; i++) {
        result[i] = create(PATHS[i]);
        if (result[i] == 0) {
            (void)cgov_sc(SYS_FS_CLOSE, (u64)fd_word, 0, 0, 0, 0, 0, 0, 0);
            (void)cgov_sc(SYS_FS_UNLINK, EA(PATHS[i]), 0, 0, 0, 0, 0, 0, 0);
        }
    }

    frame_head[7] = (unsigned char)len;
    sysTtyWrite(0, CGOV_MAGIC, 4, &written);
    sysTtyWrite(0, frame_head + 4, 4, &written);
    sysTtyWrite(0, (const void *)result, len, &written);

    for (i = 0; i < 2; i++) {
        if (create(PATHS[i]) != 0)
            continue;
        (void)cgov_sc(SYS_FS_WRITE, (u64)fd_word, EA(frame_head), 8, EA(&nwritten), 0, 0, 0, 0);
        (void)cgov_sc(SYS_FS_WRITE, (u64)fd_word, EA(result), len, EA(&nwritten), 0, 0, 0, 0);
        (void)cgov_sc(SYS_FS_CLOSE, (u64)fd_word, 0, 0, 0, 0, 0, 0, 0);
        break;
    }
    return 0;
}
