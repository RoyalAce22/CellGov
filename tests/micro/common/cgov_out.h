/* Shared CGOV frame output for the PS3 variant of a microtest.
 *
 * CGOV_OUT_FILE_WRITE(payload, len) is empty unless the build defines
 * CGOV_OUT_FILE to a path string. It then writes the same frame the
 * microtest puts on the TTY -- the CGOV magic, the big-endian length,
 * the payload -- to that file through raw LV2 syscalls: sys_fs_open
 * (801) for write with create and truncate, sys_fs_write (803) and
 * sys_fs_close (804). Raw syscalls keep the PS3 variant off the
 * toolchain's fs library, which the -nostartfiles link never
 * initializes. Each call rewrites the file, so it holds the last frame
 * the program wrote; the TTY carries every frame.
 *
 * The default build leaves the macro empty, so the reference ELFs the
 * emulator baselines were recorded from do not change by one byte.
 */

#ifndef CGOV_OUT_H
#define CGOV_OUT_H

#ifdef CGOV_OUT_FILE

#define CGOV_OUT_SYS_FS_OPEN  801
#define CGOV_OUT_SYS_FS_WRITE 803
#define CGOV_OUT_SYS_FS_CLOSE 804

/* CELL_FS_O_WRONLY (0o1) | CELL_FS_O_CREAT (0o100) | CELL_FS_O_TRUNC
 * (0o1000); the values cellgov_ps3_abi::lv2::fs carries. */
#define CGOV_OUT_OPEN_FLAGS 0x241
#define CGOV_OUT_OPEN_MODE  0666

/* One LV2 system call. The kernel may change every argument register
 * r3-r10 and the number register r11, as well as r0, r12, the link,
 * count and fixed-point exception registers and condition fields 0, 1,
 * 5, 6 and 7, so each is an output or a clobber here: the register
 * contract of the toolchain's own ppu-lv2.h macros. With r4-r10 marked
 * as inputs only, the compiler may keep a value in one of them across
 * the call, and the next call then passes whatever the kernel left. */
static inline long long cgov_out_sc(unsigned long long num,
                                    unsigned long long a,
                                    unsigned long long b,
                                    unsigned long long c,
                                    unsigned long long d,
                                    unsigned long long e,
                                    unsigned long long f)
{
    register unsigned long long r3 __asm__("3") = a;
    register unsigned long long r4 __asm__("4") = b;
    register unsigned long long r5 __asm__("5") = c;
    register unsigned long long r6 __asm__("6") = d;
    register unsigned long long r7 __asm__("7") = e;
    register unsigned long long r8 __asm__("8") = f;
    register unsigned long long r9 __asm__("9") = 0;
    register unsigned long long r10 __asm__("10") = 0;
    register unsigned long long r11 __asm__("11") = num;
    __asm__ volatile (
        "sc\n"
        : "+r"(r3), "+r"(r4), "+r"(r5), "+r"(r6), "+r"(r7), "+r"(r8),
          "+r"(r9), "+r"(r10), "+r"(r11)
        :
        : "r0", "r12", "lr", "ctr", "xer", "cr0", "cr1", "cr5", "cr6",
          "cr7", "memory"
    );
    return (long long)r3;
}

#define cgov_out_sc6(num, a, b, c, d, e, f) cgov_out_sc((num), (a), (b), (c), (d), (e), (f))
#define cgov_out_sc4(num, a, b, c, d) cgov_out_sc((num), (a), (b), (c), (d), 0, 0)
#define cgov_out_sc1(num, a) cgov_out_sc((num), (a), 0, 0, 0, 0, 0)

#define CGOV_OUT_EA(p) ((unsigned long long)(unsigned long)(p))

static void cgov_out_file_write(const void *payload, unsigned int len)
{
    static const char cgov_out_magic[4] = { 'C', 'G', 'O', 'V' };
    static const char cgov_out_path[] = CGOV_OUT_FILE;
    unsigned char len_be[4];
    int fd = -1;
    unsigned long long nwrite = 0;

    len_be[0] = (len >> 24) & 0xFF;
    len_be[1] = (len >> 16) & 0xFF;
    len_be[2] = (len >>  8) & 0xFF;
    len_be[3] = (len      ) & 0xFF;

    if (cgov_out_sc6(CGOV_OUT_SYS_FS_OPEN, CGOV_OUT_EA(cgov_out_path),
                     CGOV_OUT_OPEN_FLAGS, CGOV_OUT_EA(&fd),
                     CGOV_OUT_OPEN_MODE, 0, 0) != 0)
        return;
    cgov_out_sc4(CGOV_OUT_SYS_FS_WRITE, (unsigned long long)fd,
                 CGOV_OUT_EA(cgov_out_magic), 4, CGOV_OUT_EA(&nwrite));
    cgov_out_sc4(CGOV_OUT_SYS_FS_WRITE, (unsigned long long)fd,
                 CGOV_OUT_EA(len_be), 4, CGOV_OUT_EA(&nwrite));
    cgov_out_sc4(CGOV_OUT_SYS_FS_WRITE, (unsigned long long)fd,
                 CGOV_OUT_EA(payload), len, CGOV_OUT_EA(&nwrite));
    cgov_out_sc1(CGOV_OUT_SYS_FS_CLOSE, (unsigned long long)fd);
}

#define CGOV_OUT_FILE_WRITE(payload, len) cgov_out_file_write((payload), (len))

#else /* !CGOV_OUT_FILE */

#define CGOV_OUT_FILE_WRITE(payload, len) ((void)0)

#endif /* CGOV_OUT_FILE */

#endif /* CGOV_OUT_H */
