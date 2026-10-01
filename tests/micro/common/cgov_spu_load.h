/* SPU image loading for the PS3 variant of a microtest.
 *
 * CGOV_SPU_IMAGE_OPEN(image, path) is sysSpuImageOpen(image, path)
 * unless the build defines CGOV_PS3_USRDIR, the console directory the
 * package deploys into (package_ps3.sh defines it for the console
 * relink). It then reads the file named by the last component of path
 * from that directory with raw LV2 file syscalls and imports it from
 * memory with sysSpuImageImport.
 *
 * Two answers measured on a retail console (CECH-2001A, 4.93 CEX)
 * make the console variant differ:
 *   - webMAN's /play.ps3 does not map /app_home to the EBOOT's USRDIR:
 *     sys_spu_image_open on /app_home/<name> answers CELL_ENOENT;
 *   - sys_spu_image_open on the deployed file by its absolute path
 *     answers CELL_EAUTHFAIL, because the retail kernel refuses an
 *     unsigned SPU ELF. The same bytes imported from memory run.
 *
 * The default build leaves the macro as the plain open, so the
 * reference ELFs the emulator baselines were recorded from do not
 * change by one byte.
 */

#ifndef CGOV_SPU_LOAD_H
#define CGOV_SPU_LOAD_H

#ifdef CGOV_PS3_USRDIR

#include <lv2/spu.h>

#include "cgov_out.h"

#define CGOV_SPU_SYS_FS_READ 802

/* Room for any SPU program: the local store is 256 KiB, and the ELF
 * adds headers to that. */
#define CGOV_SPU_ELF_MAX (512 * 1024)

static unsigned char cgov_spu_elf[CGOV_SPU_ELF_MAX] __attribute__((aligned(128)));

static int cgov_spu_image_open(sysSpuImage *image, const char *path)
{
    static const char usrdir[] = CGOV_PS3_USRDIR;
    static char full[256];
    const char *name = path;
    unsigned int at = 0;
    int fd = -1;
    unsigned long long nread = 0;
    long long rc;

    for (const char *p = path; *p != '\0'; ++p)
        if (*p == '/')
            name = p + 1;
    for (const char *p = usrdir; *p != '\0' && at < sizeof(full) - 1; ++p)
        full[at++] = *p;
    if (at < sizeof(full) - 1)
        full[at++] = '/';
    for (const char *p = name; *p != '\0' && at < sizeof(full) - 1; ++p)
        full[at++] = *p;
    full[at] = '\0';

    /* CELL_FS_O_RDONLY is 0. */
    rc = cgov_out_sc6(CGOV_OUT_SYS_FS_OPEN, CGOV_OUT_EA(full), 0,
                      CGOV_OUT_EA(&fd), 0, 0, 0);
    if (rc != 0)
        return (int)rc;
    rc = cgov_out_sc4(CGOV_SPU_SYS_FS_READ, (unsigned long long)fd,
                      CGOV_OUT_EA(cgov_spu_elf), sizeof(cgov_spu_elf),
                      CGOV_OUT_EA(&nread));
    cgov_out_sc1(CGOV_OUT_SYS_FS_CLOSE, (unsigned long long)fd);
    if (rc != 0)
        return (int)rc;
    if (nread == 0 || nread >= sizeof(cgov_spu_elf))
        return -1;
    return sysSpuImageImport(image, cgov_spu_elf, 0);
}

#define CGOV_SPU_IMAGE_OPEN(image, path) cgov_spu_image_open((image), (path))

#else /* !CGOV_PS3_USRDIR */

#define CGOV_SPU_IMAGE_OPEN(image, path) sysSpuImageOpen((image), (path))

#endif /* CGOV_PS3_USRDIR */

#endif /* CGOV_SPU_LOAD_H */
