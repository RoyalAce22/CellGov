/* PPU probe: the open-flag bits, the path and name limits and the
 * record sizes of the file-system calls.
 *
 * The probe makes one directory under /dev_hdd0/tmp with mode 0700 and
 * files in it with mode 0600, the modes lv2_error_codes_fs and
 * lv2_fs_answers ran clean with, and removes all of them before it
 * writes its frame. Outside that directory it only stats one file under
 * /dev_flash.
 *
 * Every call is one raw `sc`; a u64 return-code word holds r3 as the
 * kernel left it. A slot never written holds 0xEEEEEEEEEEEEEEEE.
 *
 * Payload: big-endian u64 words, one per slot.
 *  setup
 *    0  811 mkdir D rc   1  801 create F rc   2  803 write 4 bytes rc
 *  A. opens of F (4 bytes long), each closed after
 *    3  O_RDONLY rc            4  803 write 1 byte on it rc
 *    5  O_WRONLY rc            6  802 read 1 byte on it rc
 *    7  O_RDWR rc
 *    8  O_ACCMODE (3) rc
 *    9  O_CREAT|O_EXCL rc
 *   10  O_WRONLY|O_APPEND rc  11  803 write 1 byte rc
 *   12  818 CUR position after it   13  F's size after the close
 *   14  O_WRONLY|O_TRUNC rc   15  F's size after the close
 *   16  O_WRONLY|O_CREAT|O_TRUNC of a new G rc   17  G's st_mode
 *   18..29  O_RDONLY with one more bit, rc: 0x4 0x8 0x10 0x20 0x100
 *           0x800 0x1000 0x2000 0x4000 0x8000 0x10000 0x20000
 *  B. 808 stat of paths that do not exist, rc
 *   30..33  path of 1021, 1022, 1023 and 1024 bytes before the NUL
 *   34  name of 255 bytes in D    35  name of 256 bytes in D
 *  C. records over 0xEE-filled buffers
 *   36  808 stat F rc   37  bytes written (index of the last changed
 *       byte + 1)   38  st_mode   39  st_size   40  st_blksize
 *   41  809 fstat F rc  42  bytes written
 *   43  805 opendir D rc
 *   44..55  4 readdir calls, 3 words each: rc << 32 | nread,
 *           bytes written, d_type << 8 | d_namlen
 *   56  807 closedir rc
 *  D. /dev_flash
 *   57  808 stat of FLASH_FILE rc   58  st_mode   59  st_blksize
 *   60  808 stat of D rc            61  st_mode
 *   62  cleanup: unlink F rc        63  rmdir D rc
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
#define O_ACCMODE 0x3
#define O_CREAT  0x40
#define O_EXCL   0x80
#define O_TRUNC  0x200
#define O_APPEND 0x400
#define MODE_FILE 0x180
#define MODE_DIR  0x1C0
#define SEEK_CUR 1

#define WORDS 64
#define SENTINEL 0xEEEEEEEEEEEEEEEEULL

static const char DIR_D[] = "/dev_hdd0/tmp/cgov_fsc_dir";
static const char FILE_F[] = "/dev_hdd0/tmp/cgov_fsc_dir/f";
static const char FILE_G[] = "/dev_hdd0/tmp/cgov_fsc_dir/g";
static const char FLASH_FILE[] = "/dev_flash/sys/external/liblv2.sprx";

static const unsigned int EXTRA_BITS[12] = {
    0x4, 0x8, 0x10, 0x20, 0x100, 0x800, 0x1000, 0x2000, 0x4000, 0x8000, 0x10000, 0x20000,
};
static const unsigned int PATH_LENS[4] = { 1021, 1022, 1023, 1024 };

static volatile u64 result[WORDS] __attribute__((aligned(128)));
static volatile int fd_word __attribute__((aligned(8)));
static volatile u64 out_word __attribute__((aligned(8)));
static volatile unsigned char buf[0x200] __attribute__((aligned(16)));
static char path[1100] __attribute__((aligned(16)));
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

/* Open `p`; the fd, or -1 when the open refused. */
static int fs_open(const char *p, u64 flags, u64 mode, u64 *rc)
{
    u64 r;
    fd_word = -1;
    r = sc(SYS_FS_OPEN, EA(p), flags, EA(&fd_word), mode);
    if (rc)
        *rc = r;
    return r == 0 ? fd_word : -1;
}

static void close_fd(int fd)
{
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

static void fill(void)
{
    memset((void *)buf, 0xEE, sizeof(buf));
}

/* The index of the last byte of `buf` a call changed, plus one. */
static u64 written(void)
{
    unsigned i = sizeof(buf);
    while (i > 0 && buf[i - 1] == 0xEE)
        i--;
    return i;
}

/* 808 stat of `p` into a filled `buf`: its rc. */
static u64 stat_into(const char *p)
{
    fill();
    return sc(SYS_FS_STAT, EA(p), EA(buf), 0, 0);
}

static void clean(void)
{
    (void)sc(SYS_FS_UNLINK, EA(FILE_F), 0, 0, 0);
    (void)sc(SYS_FS_UNLINK, EA(FILE_G), 0, 0, 0);
    (void)sc(SYS_FS_RMDIR, EA(DIR_D), 0, 0, 0);
}

/* `path` = D + "/" + `n` bytes of 'n'. */
static void name_path(unsigned n)
{
    unsigned at = strlen(DIR_D);
    memcpy(path, DIR_D, at);
    path[at++] = '/';
    memset(path + at, 'n', n);
    path[at + n] = 0;
}

/* `path` = D + "/" + components of 'a' (none longer than 200 bytes)
 * making `len` bytes before the NUL. */
static void long_path(unsigned len)
{
    unsigned at = strlen(DIR_D);
    memcpy(path, DIR_D, at);
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
    unsigned int i;
    int fd, dir_fd;
    u64 rc;

    for (i = 0; i < WORDS; i++)
        result[i] = SENTINEL;

    /* Whatever an earlier run left goes first. */
    clean();
    result[0] = sc(SYS_FS_MKDIR, EA(DIR_D), MODE_DIR, 0, 0);
    fd = fs_open(FILE_F, O_WRONLY | O_CREAT | O_EXCL, MODE_FILE, &rc);
    result[1] = rc;
    if (fd >= 0) {
        out_word = SENTINEL;
        result[2] = sc(SYS_FS_WRITE, (u64)fd, EA(WXYZ), 4, EA(&out_word));
        close_fd(fd);
    }

    /* A. */
    fd = fs_open(FILE_F, O_RDONLY, 0, &rc);
    result[3] = rc;
    if (fd >= 0) {
        out_word = SENTINEL;
        result[4] = sc(SYS_FS_WRITE, (u64)fd, EA(WXYZ), 1, EA(&out_word));
        close_fd(fd);
    }
    fd = fs_open(FILE_F, O_WRONLY, 0, &rc);
    result[5] = rc;
    if (fd >= 0) {
        out_word = SENTINEL;
        result[6] = sc(SYS_FS_READ, (u64)fd, EA(buf), 1, EA(&out_word));
        close_fd(fd);
    }
    fd = fs_open(FILE_F, O_RDWR, 0, &rc);
    result[7] = rc;
    close_fd(fd);
    fd = fs_open(FILE_F, O_ACCMODE, 0, &rc);
    result[8] = rc;
    close_fd(fd);
    fd = fs_open(FILE_F, O_CREAT | O_EXCL, MODE_FILE, &rc);
    result[9] = rc;
    close_fd(fd);
    fd = fs_open(FILE_F, O_WRONLY | O_APPEND, 0, &rc);
    result[10] = rc;
    if (fd >= 0) {
        out_word = SENTINEL;
        result[11] = sc(SYS_FS_WRITE, (u64)fd, EA(WXYZ), 1, EA(&out_word));
        out_word = SENTINEL;
        (void)sc(SYS_FS_LSEEK, (u64)fd, 0, SEEK_CUR, EA(&out_word));
        result[12] = out_word;
        close_fd(fd);
    }
    if (stat_into(FILE_F) == 0)
        result[13] = be64(buf + 36);
    fd = fs_open(FILE_F, O_WRONLY | O_TRUNC, 0, &rc);
    result[14] = rc;
    close_fd(fd);
    if (stat_into(FILE_F) == 0)
        result[15] = be64(buf + 36);
    fd = fs_open(FILE_G, O_WRONLY | O_CREAT | O_TRUNC, MODE_FILE, &rc);
    result[16] = rc;
    close_fd(fd);
    if (stat_into(FILE_G) == 0)
        result[17] = be32(buf);
    (void)sc(SYS_FS_UNLINK, EA(FILE_G), 0, 0, 0);
    for (i = 0; i < 12; i++) {
        fd = fs_open(FILE_F, O_RDONLY | EXTRA_BITS[i], 0, &rc);
        result[18 + i] = rc;
        close_fd(fd);
    }

    /* B. */
    for (i = 0; i < 4; i++) {
        long_path(PATH_LENS[i]);
        result[30 + i] = stat_into(path);
    }
    name_path(255);
    result[34] = stat_into(path);
    name_path(256);
    result[35] = stat_into(path);

    /* C. */
    result[36] = stat_into(FILE_F);
    result[37] = written();
    if (result[36] == 0) {
        result[38] = be32(buf);
        result[39] = be64(buf + 36);
        result[40] = be64(buf + 44);
    }
    fd = fs_open(FILE_F, O_RDONLY, 0, 0);
    if (fd >= 0) {
        fill();
        result[41] = sc(SYS_FS_FSTAT, (u64)fd, EA(buf), 0, 0);
        result[42] = written();
        close_fd(fd);
    }
    fd_word = -1;
    result[43] = sc(SYS_FS_OPENDIR, EA(DIR_D), EA(&fd_word), 0, 0);
    if (result[43] == 0) {
        dir_fd = fd_word;
        for (i = 0; i < 4; i++) {
            fill();
            out_word = SENTINEL;
            rc = sc(SYS_FS_READDIR, (u64)dir_fd, EA(buf), EA(&out_word), 0);
            result[44 + 3 * i] = (rc << 32) | (out_word & 0xFFFFFFFFULL);
            result[45 + 3 * i] = written();
            result[46 + 3 * i] = ((u64)buf[0] << 8) | buf[1];
            if (rc != 0 || out_word == 0)
                break;
        }
        result[56] = sc(SYS_FS_CLOSEDIR, (u64)dir_fd, 0, 0, 0);
    }

    /* D. */
    result[57] = stat_into(FLASH_FILE);
    if (result[57] == 0) {
        result[58] = be32(buf);
        result[59] = be64(buf + 44);
    }
    result[60] = stat_into(DIR_D);
    if (result[60] == 0)
        result[61] = be32(buf);

    result[62] = sc(SYS_FS_UNLINK, EA(FILE_F), 0, 0, 0);
    result[63] = sc(SYS_FS_RMDIR, EA(DIR_D), 0, 0, 0);

    write_frame();
    return 0;
}
