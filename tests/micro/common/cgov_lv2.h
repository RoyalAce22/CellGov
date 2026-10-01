/* LV2 system calls for the PPU microtests.
 *
 * Every helper is one `sc` under the register contract of the
 * toolchain's own ppu-lv2.h macros, the one cgov_out.h states: the
 * kernel may change every argument register r3-r10 and the number
 * register r11, as well as r0, r12, the link, count and fixed-point
 * exception registers and condition fields 0, 1, 5, 6 and 7. A helper
 * that marks r4-r10 as inputs only lets the compiler keep a live value
 * in one of them across the call; the console's kernel then overwrites
 * it, where an emulator that preserves them does not.
 *
 * Include it after the PSL1GHT headers that define s32 and u64.
 */

#ifndef CGOV_LV2_H
#define CGOV_LV2_H

static inline long long cgov_sc(u64 num, u64 a, u64 b, u64 c, u64 d,
                                u64 e, u64 f, u64 g, u64 h)
{
    register u64 r3 __asm__("3") = a;
    register u64 r4 __asm__("4") = b;
    register u64 r5 __asm__("5") = c;
    register u64 r6 __asm__("6") = d;
    register u64 r7 __asm__("7") = e;
    register u64 r8 __asm__("8") = f;
    register u64 r9 __asm__("9") = g;
    register u64 r10 __asm__("10") = h;
    register u64 r11 __asm__("11") = num;
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

#define syscall0_s32(num) \
    ((s32)cgov_sc((num), 0, 0, 0, 0, 0, 0, 0, 0))
#define syscall1_s32(num, a) \
    ((s32)cgov_sc((num), (a), 0, 0, 0, 0, 0, 0, 0))
#define syscall2_s32(num, a, b) \
    ((s32)cgov_sc((num), (a), (b), 0, 0, 0, 0, 0, 0))
#define syscall3_s32(num, a, b, c) \
    ((s32)cgov_sc((num), (a), (b), (c), 0, 0, 0, 0, 0))
#define syscall4_s32(num, a, b, c, d) \
    ((s32)cgov_sc((num), (a), (b), (c), (d), 0, 0, 0, 0))
#define syscall5_s32(num, a, b, c, d, e) \
    ((s32)cgov_sc((num), (a), (b), (c), (d), (e), 0, 0, 0))
#define syscall6_s32(num, a, b, c, d, e, f) \
    ((s32)cgov_sc((num), (a), (b), (c), (d), (e), (f), 0, 0))
#define syscall8_s32(num, a, b, c, d, e, f, g, h) \
    ((s32)cgov_sc((num), (a), (b), (c), (d), (e), (f), (g), (h)))
#define syscall1_noreturn(num, a) \
    ((void)cgov_sc((num), (a), 0, 0, 0, 0, 0, 0, 0))

#define CGOV_SYS_PPU_THREAD_CREATE 52
#define CGOV_SYS_PPU_THREAD_START  53

/* sys_ppu_thread_create as the system library performs it: syscall 52
 * creates the thread stopped, and syscall 53 starts it. A thread
 * created by 52 alone never runs on the console, so a join on it never
 * returns. The arguments are syscall 52's: (thread_id*, param*, arg,
 * unk, prio, stacksize, flags, threadname*). Returns 52's error, else
 * 53's. */
static inline s32 cgov_ppu_thread_create(unsigned long long *tid, u64 param, u64 arg,
                                         u64 unk, u64 prio, u64 stacksize,
                                         u64 flags, u64 name)
{
    s32 ret = syscall8_s32(CGOV_SYS_PPU_THREAD_CREATE,
                           (u64)(unsigned long)tid, param, arg, unk, prio,
                           stacksize, flags, name);
    if (ret != 0)
        return ret;
    return syscall2_s32(CGOV_SYS_PPU_THREAD_START, *tid, 0);
}

#endif /* CGOV_LV2_H */
