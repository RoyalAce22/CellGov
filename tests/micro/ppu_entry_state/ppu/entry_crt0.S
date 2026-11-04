/*
 * The ppu_entry_state start file: tests/micro/common/crt0.S with a
 * stub in front that stores every register the main thread starts
 * with, before any other instruction writes one.
 *
 * The stub saves r9 below the stack pointer, points r9 at
 * cgov_entry_main (ppu/main.c), and stores the GPRs, CR, XER, CTR, LR,
 * VRSAVE, the FPRs, FPSCR, the VRs and VSCR there in the layout
 * ppu/main.c gives; f0 and v0 are stored before mffs and mfvscr reuse
 * them. The common start code then runs unchanged.
 */

    .section .rodata
    .globl  _start
    .type   _start, @object
    .align  3
_start:
    .long   __cg_entry      /* code address (32-bit) */
    .long   __cg_entry      /* TOC placeholder       */
    .long   0, 0, 0, 0

    .section .text
    .globl  __cg_entry
    .type   __cg_entry, @function
__cg_entry:
    std     %r9, -8(%r1)
    lis     %r9, cgov_entry_main@ha
    addi    %r9, %r9, cgov_entry_main@l
    .irp n, 0,1,2,3,4,5,6,7,8,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    std     %r\n, (8 * \n)(%r9)
    .endr
    ld      %r0, -8(%r1)
    std     %r0, 72(%r9)
    mfcr    %r0
    std     %r0, 256(%r9)
    mfxer   %r0
    std     %r0, 264(%r9)
    mfctr   %r0
    std     %r0, 272(%r9)
    mflr    %r0
    std     %r0, 280(%r9)
    mfspr   %r0, 256
    std     %r0, 288(%r9)
    .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    stfd    %f\n, (304 + 8 * \n)(%r9)
    .endr
    mffs    %f0
    stfd    %f0, 296(%r9)
    .irp n, 0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23,24,25,26,27,28,29,30,31
    li      %r0, (560 + 16 * \n)
    stvx    %v\n, %r9, %r0
    .endr
    mfvscr  %v0
    li      %r0, 1072
    stvx    %v0, %r9, %r0

    /* Load TOC base from a data word placed after the code.
     * The build script patches __cg_tocval with .got + 0x8000
     * after linking. */
    bcl     20, 31, 1f
1:
    mflr    %r2
    lwz     %r2, (__cg_tocval - 1b)(%r2)

    /* Save stack end for PSL1GHT */
    lis     %r9, __libc_stack_end@ha
    stw     %r1, __libc_stack_end@l(%r9)

    /* Standard PPC64 function prologue (32-bit stores are fine
     * since the stack is in the low 32-bit address range). */
    mflr    %r0
    stw     %r0, 4(%r1)
    stwu    %r1, -128(%r1)
    li      %r0, 0
    stw     %r0, 0(%r1)

    /* Call main(argc=0, argv=NULL). The microtest uses direct
     * syscalls (sys_tty_write, sys_process_exit) so it does not
     * need the full PSL1GHT constructor chain. */
    li      %r3, 0
    li      %r4, 0
    bl      main
    nop

    /* sys_process_exit(r3) -- syscall 22 */
    li      %r11, 22
    sc
    b       .

    .align  2
    .globl  __cg_tocval
__cg_tocval:
    .long   0xDEADBEEF          /* patched by build script */

    /* Stubs for _init/_fini -- PSL1GHT doesn't provide crti.o.
     * Nothing calls them; they exist to satisfy the link. */
    .section .text
    .globl  _init
    .type   _init, @function
_init:
    blr

    .globl  _fini
    .type   _fini, @function
_fini:
    blr

    .section .bss
    .globl  __libc_stack_end
    .type   __libc_stack_end, @object
    .align  2
__libc_stack_end:
    .space  8

