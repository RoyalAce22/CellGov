/* PPU probe: what the file-system calls answer on success paths -- the
 * first fds, a write, readdir order, stat words and lseek.
 *
 * The probe makes one directory under /dev_hdd0/tmp with mode 0700 and
 * four files in it with mode 0600, the modes lv2_error_codes_fs ran
 * clean with, and removes all of them before it writes its frame. It
 * touches no other path.
 *
 * Every call is one raw `sc`. A u64 return-code word holds r3 as the
 * kernel left it. Output words a call did not write keep the sentinel
 * 0xEEEEEEEEEEEEEEEE.
 *
 * Payload: 64 big-endian u64 words, 512 bytes.
 *
 *  fds
 *    0  fd of open #1 (dir/a, read-only)
 *    1  fd of open #2 (dir/b, read-only)
 *    2  fd of open #3 (dir/c, read-only)
 *  write
 *    3  801 open dir/w O_RDWR|O_CREAT rc        4  that fd
 *    5  803 write 4 bytes to w rc               6  nwrite
 *    7  803 write 0 bytes to fd #1 rc           8  nwrite
 *    9  803 write 4 bytes to fd #1 rc          10  nwrite
 *  stat
 *   11  809 fstat w rc   12  st_mode   13  st_size   14  st_blksize
 *   15  808 stat dir/w rc 16  st_mode  17  st_size   18  st_blksize
 *   19  808 stat dir rc   20  st_mode  21  st_size   22  st_blksize
 *  lseek on w (4 bytes long)
 *   23  SET 1 rc   24  pos
 *   25  CUR 1 rc   26  pos
 *   27  END 0 rc   28  pos
 *   29  SET 10 (past the end) rc   30  pos
 *   31  SET -1 rc  32  pos
 *   33  whence 3 rc
 *  read
 *   34  818 SET 0 rc
 *   35  802 read 2 bytes into an unmapped buffer rc   36  nread
 *   37  818 CUR 0 pos after that read
 *   38  802 read 2 bytes rc   39  nread   40  the two bytes
 *  readdir of dir
 *   41  805 opendir rc   42  dir fd
 *   43..58  8 entries, two words each:
 *           word 0  rc << 32 | nread (low 32 bits)
 *           word 1  d_type << 56 | d_namlen << 48 | first 6 name bytes
 *   59  807 closedir rc   60  807 second closedir rc
 *  readdir error order, fd 0x7FFF
 *   61  dirent NULL, nread NULL
 *   62  dirent valid, nread NULL
 *   63  dirent NULL, nread valid
 *
 * Setup and cleanup answers are not in the payload: a failed mkdir or
 * create shows as an open that refuses.
 */

#include <string.h>

#include <sys/process.h>
#include <sys/tty.h>

#include "cgov_lv2.h"
#include "cgov_out.h"

SYS_PROCESS_PARAM(1001, 0x10000)

#define SYS_FS_OPEN     801
#define SYS_FS_READ     802
#define SYS_FS_WRITE    803
#define SYS_FS_CLOSE    804
#define SYS_FS_OPENDIR  805
#define SYS_FS_READDIR  806
#define SYS_FS_CLOSEDIR 807
#define SYS_FS_STAT     808
#define SYS_FS_FSTAT    809
#define SYS_FS_MKDIR    811
#define SYS_FS_RMDIR    813
#define SYS_FS_UNLINK   814
#define SYS_FS_LSEEK    818

#define O_RDONLY 0x0
#define O_WRONLY 0x1
#define O_RDWR   0x2
#define O_CREAT  0x40
#define O_EXCL   0x80
#define MODE_FILE 0x180
#define MODE_DIR  0x1C0

#define SEEK_SET 0
#define SEEK_CUR 1
#define SEEK_END 2

/* Page 0 is never mapped in a game process. */
#define UNMAPPED 0x100

#define WORDS 64
#define SENTINEL 0xEEEEEEEEEEEEEEEEULL
#define BAD_FD 0x7FFF

static const char TEST_DIR[] = "/dev_hdd0/tmp/cgov_fsans_dir";
static const char FILE_A[] = "/dev_hdd0/tmp/cgov_fsans_dir/a";
static const char FILE_B[] = "/dev_hdd0/tmp/cgov_fsans_dir/b";
static const char FILE_C[] = "/dev_hdd0/tmp/cgov_fsans_dir/c";
static const char FILE_W[] = "/dev_hdd0/tmp/cgov_fsans_dir/w";

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static volatile int fd_word __attribute__((aligned(8)));
static volatile u64 out_word __attribute__((aligned(8)));
static volatile unsigned char stat_buf[64] __attribute__((aligned(16)));
static volatile unsigned char dirent[264] __attribute__((aligned(16)));
static volatile unsigned char bytes[8] __attribute__((aligned(16)));
static const unsigned char WXYZ[4] = { 'W', 'X', 'Y', 'Z' };

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

/* Open `path`; the fd, or -1 when the open refused. */
static int fs_open(const char *path, u64 flags, u64 mode, u64 *rc)
{
    u64 r;
    fd_word = -1;
    r = sc(SYS_FS_OPEN, EA(path), flags, EA(&fd_word), mode);
    if (rc)
        *rc = r;
    return r == 0 ? fd_word : -1;
}

static void create_empty(const char *path)
{
    int fd = fs_open(path, O_WRONLY | O_CREAT | O_EXCL, MODE_FILE, 0);
    if (fd >= 0)
        (void)sc(SYS_FS_CLOSE, (u64)fd, 0, 0, 0);
}

static u64 be32(volatile unsigned char *p)
{
    return ((u64)p[0] << 24) | ((u64)p[1] << 16) | ((u64)p[2] << 8) | p[3];
}

static u64 be64(volatile unsigned char *p)
{
    return (be32(p) << 32) | be32(p + 4);
}

/* The stat words: st_mode, st_size, st_blksize. */
static void stat_words(unsigned slot)
{
    result[slot] = be32(stat_buf);
    result[slot + 1] = be64(stat_buf + 40);
    result[slot + 2] = be64(stat_buf + 48);
}

static void seek(int fd, long long offset, u64 whence, unsigned slot)
{
    out_word = SENTINEL;
    result[slot] = sc(SYS_FS_LSEEK, (u64)fd, (u64)offset, whence, EA(&out_word));
    result[slot + 1] = out_word;
}

static void clean(void)
{
    (void)sc(SYS_FS_UNLINK, EA(FILE_A), 0, 0, 0);
    (void)sc(SYS_FS_UNLINK, EA(FILE_B), 0, 0, 0);
    (void)sc(SYS_FS_UNLINK, EA(FILE_C), 0, 0, 0);
    (void)sc(SYS_FS_UNLINK, EA(FILE_W), 0, 0, 0);
    (void)sc(SYS_FS_RMDIR, EA(TEST_DIR), 0, 0, 0);
}

int main(void)
{
    unsigned int i;
    int fd_a, fd_b, fd_c, fd_w = -1, dir_fd = -1;
    u64 rc;

    for (i = 0; i < WORDS; i++)
        result[i] = SENTINEL;

    /* Whatever an earlier run left goes first. */
    clean();
    (void)sc(SYS_FS_MKDIR, EA(TEST_DIR), MODE_DIR, 0, 0);
    create_empty(FILE_B);
    create_empty(FILE_A);
    create_empty(FILE_C);

    fd_a = fs_open(FILE_A, O_RDONLY, 0, 0);
    fd_b = fs_open(FILE_B, O_RDONLY, 0, 0);
    fd_c = fs_open(FILE_C, O_RDONLY, 0, 0);
    result[0] = (u64)(long long)fd_a;
    result[1] = (u64)(long long)fd_b;
    result[2] = (u64)(long long)fd_c;

    fd_w = fs_open(FILE_W, O_RDWR | O_CREAT, MODE_FILE, &rc);
    result[3] = rc;
    result[4] = (u64)(long long)fd_w;
    if (fd_w >= 0) {
        out_word = SENTINEL;
        result[5] = sc(SYS_FS_WRITE, (u64)fd_w, EA(WXYZ), 4, EA(&out_word));
        result[6] = out_word;
    }
    if (fd_a >= 0) {
        out_word = SENTINEL;
        result[7] = sc(SYS_FS_WRITE, (u64)fd_a, EA(WXYZ), 0, EA(&out_word));
        result[8] = out_word;
        out_word = SENTINEL;
        result[9] = sc(SYS_FS_WRITE, (u64)fd_a, EA(WXYZ), 4, EA(&out_word));
        result[10] = out_word;
    }

    if (fd_w >= 0) {
        memset((void *)stat_buf, 0xEE, sizeof(stat_buf));
        result[11] = sc(SYS_FS_FSTAT, (u64)fd_w, EA(stat_buf), 0, 0);
        stat_words(12);
    }
    memset((void *)stat_buf, 0xEE, sizeof(stat_buf));
    result[15] = sc(SYS_FS_STAT, EA(FILE_W), EA(stat_buf), 0, 0);
    stat_words(16);
    memset((void *)stat_buf, 0xEE, sizeof(stat_buf));
    result[19] = sc(SYS_FS_STAT, EA(TEST_DIR), EA(stat_buf), 0, 0);
    stat_words(20);

    if (fd_w >= 0) {
        seek(fd_w, 1, SEEK_SET, 23);
        seek(fd_w, 1, SEEK_CUR, 25);
        seek(fd_w, 0, SEEK_END, 27);
        seek(fd_w, 10, SEEK_SET, 29);
        seek(fd_w, -1, SEEK_SET, 31);
        out_word = SENTINEL;
        result[33] = sc(SYS_FS_LSEEK, (u64)fd_w, 0, 3, EA(&out_word));

        out_word = SENTINEL;
        result[34] = sc(SYS_FS_LSEEK, (u64)fd_w, 0, SEEK_SET, EA(&out_word));
        out_word = SENTINEL;
        result[35] = sc(SYS_FS_READ, (u64)fd_w, UNMAPPED, 2, EA(&out_word));
        result[36] = out_word;
        out_word = SENTINEL;
        (void)sc(SYS_FS_LSEEK, (u64)fd_w, 0, SEEK_CUR, EA(&out_word));
        result[37] = out_word;
        memset((void *)bytes, 0, sizeof(bytes));
        out_word = SENTINEL;
        result[38] = sc(SYS_FS_READ, (u64)fd_w, EA(bytes), 2, EA(&out_word));
        result[39] = out_word;
        result[40] = ((u64)bytes[0] << 8) | bytes[1];
    }

    fd_word = -1;
    result[41] = sc(SYS_FS_OPENDIR, EA(TEST_DIR), EA(&fd_word), 0, 0);
    if (result[41] == 0) {
        dir_fd = fd_word;
        result[42] = (u64)(long long)dir_fd;
        for (i = 0; i < 8; i++) {
            unsigned n, k;
            u64 name = 0;
            memset((void *)dirent, 0, sizeof(dirent));
            out_word = SENTINEL;
            rc = sc(SYS_FS_READDIR, (u64)dir_fd, EA(dirent), EA(&out_word), 0);
            result[43 + 2 * i] = (rc << 32) | (out_word & 0xFFFFFFFFULL);
            n = dirent[1];
            for (k = 0; k < 6; k++)
                name = (name << 8) | (k < n ? dirent[2 + k] : 0);
            result[44 + 2 * i] = ((u64)dirent[0] << 56) | ((u64)n << 48) | name;
            if (rc != 0 || out_word == 0)
                break;
        }
        result[59] = sc(SYS_FS_CLOSEDIR, (u64)dir_fd, 0, 0, 0);
        result[60] = sc(SYS_FS_CLOSEDIR, (u64)dir_fd, 0, 0, 0);
    }

    result[61] = sc(SYS_FS_READDIR, BAD_FD, 0, 0, 0);
    result[62] = sc(SYS_FS_READDIR, BAD_FD, EA(dirent), 0, 0);
    out_word = SENTINEL;
    result[63] = sc(SYS_FS_READDIR, BAD_FD, 0, EA(&out_word), 0);

    if (fd_a >= 0)
        (void)sc(SYS_FS_CLOSE, (u64)fd_a, 0, 0, 0);
    if (fd_b >= 0)
        (void)sc(SYS_FS_CLOSE, (u64)fd_b, 0, 0, 0);
    if (fd_c >= 0)
        (void)sc(SYS_FS_CLOSE, (u64)fd_c, 0, 0, 0);
    if (fd_w >= 0)
        (void)sc(SYS_FS_CLOSE, (u64)fd_w, 0, 0, 0);
    clean();

    write_frame();
    return 0;
}
