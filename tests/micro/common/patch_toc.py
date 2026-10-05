"""Post-link patches for PSL1GHT ELFs.

Usage: patch_toc.py <elf_path> <readelf_path> <nm_path> [--toc-only]

Two patches:
1. TOC base: computes .got + 0x8000, writes it into __cg_tocval.
2. rldicr removal: replaces every `rldicr rN, rN, 16, 47`
   (`sldi rN, rN, 16`) with `rlwinm rN, rN, 16, 0, 15`
   (`slwi rN, rN, 16`). RPCS3 v0.0.40 mishandles rldicr, causing
   branches to garbage addresses. The two give the same result only
   when rN < 2^16 before the shift: `slwi` clears the high word that
   `sldi` keeps, so a 64-bit constant built with `lis; sldi 16; ori`
   loses its high word.

`--toc-only` applies patch 1 and leaves every rldicr in place. The
console build (common/package_ps3.sh) passes it: the console runs
rldicr correctly, and the rewrite would change what the probe sends.

The section and symbol lookups read the ELF's own section headers and
symbol table. The readelf path names the toolchain's objdump, which
cross-checks the rldicr count; the nm argument is accepted so every
build script calls the same command line.

The rldicr scan is bounded to .text's file range -- ELF headers,
.rodata, .data, and the symbol/string tables must never be
candidates for rewriting -- and the patch count is cross-checked
against a disassembly of the same predicate, before and after the
rewrite. Both patches are applied to a copy in the same directory,
which replaces the ELF only when every check passes, so a failed run
leaves the ELF as it was.

Every expected failure is a PatchTocError subclass with its own exit
code; main() prints it on one line. Any other exception is a defect in
this script and exits 70; set CGOV_PATCH_TOC_TRACEBACK=1 for its
traceback.
"""

from __future__ import annotations

import argparse
import os
import shutil
import struct
import subprocess
import sys
import tempfile
import traceback
from typing import Callable, NoReturn, Sequence

# ELF64 layout from the System V gABI: identification, then the header fields.
ELF_MAGIC = b"\x7fELF"
ELFCLASS64 = 2
ELFDATA2MSB = 2
EM_PPC64 = 21
SHT_PROGBITS = 1
SHT_SYMTAB = 2
SHN_LORESERVE = 0xFF00
STT_FUNC = 2
STT_SECTION = 3
STT_FILE = 4

EHDR = struct.Struct(">16sHHIQQQIHHHHHH")
SHDR = struct.Struct(">IIQQQQIIQQ")
SYM = struct.Struct(">IBBHQQ")
WORD = struct.Struct(">I")
DWORD = struct.Struct(">Q")

# The word crt0.S assembles into __cg_tocval before the patch.
TOC_PLACEHOLDER = 0xDEADBEEF
TOC_BIAS = 0x8000
TOC_SYMBOL = "__cg_tocval"
# crt0.S loads the TOC with lwz, so it must fit in 32 bits.
WORD_MAX = 0xFFFFFFFF

EXIT_INTERNAL = 70
EXIT_INTERRUPTED = 130
TRACEBACK_ENV = "CGOV_PATCH_TOC_TRACEBACK"


class PatchTocError(Exception):
    """A refusal this script expects; main() prints it and exits."""

    exit_code = 1


class UsageError(PatchTocError):
    exit_code = 2

    def __init__(self, detail: str) -> None:
        self.detail = detail
        super().__init__(f"usage: {detail}")


class ToolError(PatchTocError):
    exit_code = 3

    def __init__(self, tool: str, argv: Sequence[str], returncode: int | None, stderr: str) -> None:
        self.tool = tool
        self.argv = list(argv)
        self.returncode = returncode
        self.stderr = stderr
        if returncode is None:
            msg = f"cannot run {tool}"
        else:
            msg = f"{tool} exited with status {returncode}"
        tail = stderr.strip().splitlines()
        if tail:
            msg += f": {tail[-1]}"
        super().__init__(msg)


class ElfIoError(PatchTocError):
    exit_code = 4

    def __init__(self, path: str, op: str) -> None:
        self.path = path
        self.op = op
        super().__init__(f"cannot {op} {path}")


class ElfFormatError(PatchTocError):
    exit_code = 5

    def __init__(self, path: str, reason: str) -> None:
        self.path = path
        self.reason = reason
        super().__init__(f"{path} is not a usable PPC64 ELF: {reason}")


class SectionNotFound(PatchTocError):
    exit_code = 6

    def __init__(self, name: str) -> None:
        self.name = name
        super().__init__(f"no PROGBITS section named {name}")


class SymbolNotFound(PatchTocError):
    exit_code = 7

    def __init__(self, name: str) -> None:
        self.name = name
        super().__init__(f"symbol {name} not found in .symtab")


class SymbolOutsideSection(PatchTocError):
    exit_code = 8

    def __init__(self, symbol: str, section: str, vaddr: int) -> None:
        self.symbol = symbol
        self.section = section
        self.vaddr = vaddr
        super().__init__(f"symbol {symbol} at 0x{vaddr:016x} has no 4-byte word inside section {section}")


class PlaceholderMismatch(PatchTocError):
    exit_code = 9

    def __init__(self, offset: int, found: int, expected: Sequence[int]) -> None:
        self.offset = offset
        self.found = found
        self.expected = list(expected)
        wanted = " or ".join(f"0x{v:08x}" for v in self.expected)
        super().__init__(f"word at file offset 0x{offset:x} is 0x{found:08x}, expected {wanted}")


class TocOutOfRange(PatchTocError):
    exit_code = 10

    def __init__(self, toc: int) -> None:
        self.toc = toc
        super().__init__(f"TOC 0x{toc:016x} does not fit the 32-bit word crt0.S loads")


class RldicrCountMismatch(PatchTocError):
    exit_code = 11

    def __init__(self, patched: int, expected: int, stage: str) -> None:
        self.patched = patched
        self.expected = expected
        self.stage = stage
        super().__init__(
            f"{stage}: the encoding predicate counts {patched} rldicr site(s) "
            f"and objdump counts {expected} in .text"
        )


class Section:
    """One section header, with its name resolved."""

    def __init__(self, index: int, name: str, fields: tuple[int, ...]) -> None:
        self.index = index
        self.name = name
        (_, self.sh_type, _, self.addr, self.offset, self.size, self.link, _, _, _) = fields


class Symbol:
    """One .symtab entry, with its name resolved."""

    def __init__(self, name: str, fields: tuple[int, ...]) -> None:
        self.name = name
        (_, info, _, self.shndx, self.value, self.size) = fields
        self.kind = info & 0xF


class Elf:
    """The parts of a big-endian ELF64 image this script reads."""

    def __init__(self, path: str, data: bytes) -> None:
        self.path = path
        self.data = data
        self.sections = self._read_sections()
        self.symbols = self._read_symbols()

    def _fail(self, reason: str) -> NoReturn:
        raise ElfFormatError(self.path, reason)

    def _slice(self, offset: int, size: int, what: str) -> bytes:
        if offset < 0 or size < 0 or offset + size > len(self.data):
            self._fail(f"{what} at 0x{offset:x}+0x{size:x} runs past the end of the file")
        return self.data[offset : offset + size]

    def _read_sections(self) -> list[Section]:
        if len(self.data) < EHDR.size:
            self._fail("shorter than an ELF64 header")
        ident, _, machine, _, _, _, shoff, _, _, _, _, shentsize, shnum, shstrndx = EHDR.unpack_from(self.data)
        if ident[:4] != ELF_MAGIC:
            self._fail("no ELF magic")
        if ident[4] != ELFCLASS64:
            self._fail(f"EI_CLASS is {ident[4]}, not ELFCLASS64")
        if ident[5] != ELFDATA2MSB:
            self._fail(f"EI_DATA is {ident[5]}, not big-endian")
        if machine != EM_PPC64:
            self._fail(f"e_machine is {machine}, not EM_PPC64")
        if shnum == 0 or shstrndx >= shnum:
            self._fail("no section header string table")
        if shentsize != SHDR.size:
            self._fail(f"e_shentsize is {shentsize}, not {SHDR.size}")
        table = self._slice(shoff, shnum * SHDR.size, "section header table")
        raw = [SHDR.unpack_from(table, i * SHDR.size) for i in range(shnum)]
        names = raw[shstrndx]
        strtab = self._slice(names[4], names[5], "section name table")
        return [Section(i, self._string(strtab, fields[0], "section name"), fields) for i, fields in enumerate(raw)]

    def _string(self, table: bytes, offset: int, what: str) -> str:
        end = table.find(b"\0", offset)
        if offset >= len(table) or end < 0:
            self._fail(f"{what} at string offset 0x{offset:x} is not terminated")
        return table[offset:end].decode("ascii", errors="replace")

    def _read_symbols(self) -> list[Symbol]:
        symtabs = [s for s in self.sections if s.sh_type == SHT_SYMTAB]
        if not symtabs:
            return []
        symtab = symtabs[0]
        if symtab.link >= len(self.sections):
            self._fail("the .symtab string table link is out of range")
        strings = self.sections[symtab.link]
        strtab = self._slice(strings.offset, strings.size, "symbol name table")
        table = self._slice(symtab.offset, symtab.size, ".symtab")
        count = len(table) // SYM.size
        symbols = []
        for i in range(count):
            fields = SYM.unpack_from(table, i * SYM.size)
            symbols.append(Symbol(self._string(strtab, fields[0], "symbol name"), fields))
        return symbols

    def progbits(self, name: str) -> Section:
        """The one PROGBITS section named `name`, checked to lie in the file."""
        found = [s for s in self.sections if s.name == name and s.sh_type == SHT_PROGBITS]
        if not found:
            raise SectionNotFound(name)
        if len(found) > 1:
            self._fail(f"{len(found)} PROGBITS sections are named {name}")
        self._slice(found[0].offset, found[0].size, name)
        return found[0]

    def symbol(self, name: str) -> Symbol:
        """The one .symtab entry named exactly `name`."""
        found = [s for s in self.symbols if s.name == name]
        if not found:
            raise SymbolNotFound(name)
        if len(found) > 1:
            self._fail(f"{len(found)} symbols are named {name}")
        return found[0]

    def word_offset(self, sym: Symbol) -> int:
        """File offset of the 4-byte word at `sym`, inside its own section."""
        if sym.shndx == 0 or sym.shndx >= SHN_LORESERVE or sym.shndx >= len(self.sections):
            raise SymbolOutsideSection(sym.name, f"index {sym.shndx}", sym.value)
        sec = self.sections[sym.shndx]
        if sec.sh_type != SHT_PROGBITS or not sec.addr <= sym.value <= sec.addr + sec.size - WORD.size:
            raise SymbolOutsideSection(sym.name, sec.name, sym.value)
        self._slice(sec.offset, sec.size, sec.name)
        return sym.value - sec.addr + sec.offset

    def code_address(self, sym: Symbol, section: Section) -> int | None:
        """Where `sym`'s code starts in `section`, or None.

        A symbol in `section` starts at its value. Under the 64-bit
        PowerPC ELF ABI (v1), a function symbol in .opd names a
        descriptor whose first doubleword is the code address.
        """
        if sym.shndx == section.index:
            return sym.value
        if sym.kind != STT_FUNC or sym.shndx >= len(self.sections):
            return None
        opd = self.sections[sym.shndx]
        offset = sym.value - opd.addr + opd.offset
        if opd.name != ".opd" or not opd.offset <= offset <= opd.offset + opd.size - DWORD.size:
            return None
        if offset + DWORD.size > len(self.data):
            return None
        return DWORD.unpack_from(self.data, offset)[0]

    def locate(self, section: Section, vaddr: int) -> str:
        """`name+0xoff` of the nearest function or label at or before `vaddr` in `section`."""
        best_name, best_start = None, -1
        for s in self.symbols:
            if not s.name or s.kind in (STT_SECTION, STT_FILE):
                continue
            start = self.code_address(s, section)
            if start is not None and best_start < start <= vaddr:
                best_name, best_start = s.name, start
        if best_name is None:
            return "?"
        return f"{best_name}+0x{vaddr - best_start:x}"


def matches_predicate(word: int) -> bool:
    """The exact shape patch 2 rewrites: rldicr rN, rN, 16, 47."""
    # opcode=30 (bits 0-5), sh[0:4]=16 (bits 16-20),
    # last 12 bits = 0x3E4 (me + XO + sh5 + Rc for sh=16, me=47)
    opcode = (word >> 26) & 0x3F
    sh04 = (word >> 11) & 0x1F
    tail = word & 0xFFF
    rs = (word >> 21) & 0x1F
    ra = (word >> 16) & 0x1F
    return opcode == 30 and sh04 == 16 and tail == 0x3E4 and rs == ra


def replacement(word: int) -> int:
    """rlwinm rA, rS, 16, 0, 15 (= slwi rN, rN, 16) for a matching rldicr."""
    rs = (word >> 21) & 0x1F
    ra = (word >> 16) & 0x1F
    # rlwinm encoding: opcode=21, rS, rA, SH=16, MB=0, ME=15, Rc=0
    return (21 << 26) | (rs << 21) | (ra << 16) | (16 << 11) | (0 << 6) | (15 << 1)


def run_tool(tool: str, argv: Sequence[str]) -> str:
    """stdout of `argv`; a missing tool or a non-zero exit is a ToolError."""
    try:
        done = subprocess.run(list(argv), capture_output=True, text=True, errors="replace", check=False)
    except OSError as err:
        raise ToolError(tool, argv, None, str(err)) from err
    if done.returncode != 0:
        raise ToolError(tool, argv, done.returncode, done.stderr)
    return done.stdout


def objdump_path(readelf: str) -> str:
    """The objdump beside `readelf`: only the tool name changes."""
    head, tail = os.path.split(readelf)
    stem, ext = os.path.splitext(tail)
    if not stem.endswith("readelf"):
        raise UsageError(f"cannot derive objdump from {readelf}: the name does not end in readelf")
    return os.path.join(head, stem[: -len("readelf")] + "objdump" + ext)


def count_rldicr_in_disassembly(objdump: str, elf_path: str) -> int:
    """Count `rldicr rN,rN,16,47` sites objdump decodes in .text."""
    return count_rldicr_lines(run_tool("objdump", [objdump, "-d", "--section=.text", elf_path]))


def count_rldicr_lines(disassembly: str) -> int:
    """Count `rldicr rN,rN,16,47` lines in `objdump -d` output."""
    count = 0
    for line in disassembly.splitlines():
        # "<addr>:\t<bytes>\t<mnemonic> <operands>"; some binutils
        # versions put a tab, not spaces, after the mnemonic.
        fields = line.split("\t")
        if len(fields) < 3:
            continue
        insn = " ".join(fields[2:]).split()
        if len(insn) < 2 or insn[0] != "rldicr":
            continue
        operands = insn[1].split(",")
        if len(operands) != 4:
            continue
        ra, rs, sh, me = operands
        if ra == rs and sh == "16" and me == "47":
            count += 1
    return count


def patch_rldicr(elf: Elf, data: bytearray, text: Section) -> int:
    """Rewrite matching rldicr instructions within .text; returns the count patched."""
    count = 0
    for i in range(text.offset, text.offset + text.size - 3, 4):
        word = WORD.unpack_from(data, i)[0]
        if matches_predicate(word):
            new = replacement(word)
            WORD.pack_into(data, i, new)
            vaddr = text.addr + (i - text.offset)
            print(f"  rldicr at 0x{vaddr:x} ({elf.locate(text, vaddr)}): 0x{word:08x} -> 0x{new:08x}")
            count += 1
    if count > 0:
        print(f"Patched {count} rldicr instructions")
    else:
        print("No rldicr instructions found")
    return count


def read_elf(path: str) -> bytes:
    try:
        with open(path, "rb") as f:
            return f.read()
    except OSError as err:
        raise ElfIoError(path, "read") from err


def write_candidate(path: str, data: bytes) -> str:
    """Write `data` to a new file beside `path`, durable on disk, with `path`'s mode."""
    directory = os.path.dirname(os.path.abspath(path))
    try:
        fd, tmp = tempfile.mkstemp(prefix=f".{os.path.basename(path)}.", suffix=".tmp", dir=directory)
    except OSError as err:
        raise ElfIoError(directory, "create a temporary file in") from err
    try:
        with os.fdopen(fd, "wb") as f:
            f.write(data)
            f.flush()
            os.fsync(f.fileno())
        shutil.copymode(path, tmp)
    except OSError as err:
        discard(tmp)
        raise ElfIoError(tmp, "write") from err
    return tmp


def discard(tmp: str) -> None:
    try:
        os.unlink(tmp)
    except OSError:
        pass


def patch(
    elf_path: str,
    readelf: str,
    count_sites: Callable[[str, str], int] = count_rldicr_in_disassembly,
    toc_only: bool = False,
) -> None:
    """Apply both patches to `elf_path`, or patch 1 alone when `toc_only`, all or nothing."""
    # readelf and objdump ship side by side in the toolchain bin dir;
    # build.sh passes only the former, so derive the latter.
    objdump = objdump_path(readelf)
    original = read_elf(elf_path)
    elf = Elf(elf_path, original)
    data = bytearray(original)

    got = elf.progbits(".got")
    toc = got.addr + TOC_BIAS
    print(f"GOT=0x{got.addr:x} TOC=0x{toc:08x}")
    if toc > WORD_MAX:
        raise TocOutOfRange(toc)

    file_offset = elf.word_offset(elf.symbol(TOC_SYMBOL))
    found = WORD.unpack_from(data, file_offset)[0]
    if found not in (TOC_PLACEHOLDER, toc):
        raise PlaceholderMismatch(file_offset, found, [TOC_PLACEHOLDER, toc])
    print(f"Patching 0x{toc:08x} at file offset 0x{file_offset:x}")
    WORD.pack_into(data, file_offset, toc)
    print("TOC patched")

    # Patch 2: replace rldicr with rlwinm, counted against objdump
    # before the rewrite and again on the rewritten copy.
    text = elf.progbits(".text")
    expected = count_sites(objdump, elf_path)
    if toc_only:
        print(f"rldicr left in place: {expected} site(s)")
    else:
        patched = patch_rldicr(elf, data, text)
        if patched != expected:
            raise RldicrCountMismatch(patched, expected, "before the rewrite")

    if bytes(data) == original:
        return
    tmp = write_candidate(elf_path, bytes(data))
    try:
        left = 0 if toc_only else count_sites(objdump, tmp)
        if left != 0:
            raise RldicrCountMismatch(0, left, "after the rewrite")
        try:
            os.replace(tmp, elf_path)
        except OSError as err:
            raise ElfIoError(elf_path, "replace") from err
    except BaseException:
        discard(tmp)
        raise


class _Parser(argparse.ArgumentParser):
    def error(self, message: str) -> NoReturn:
        raise UsageError(f"{self.prog} <elf_path> <readelf_path> <nm_path> [--toc-only]: {message}")


def parse_args(argv: Sequence[str]) -> argparse.Namespace:
    parser = _Parser(prog="patch_toc.py", description="Post-link TOC and rldicr patches for a PSL1GHT ELF.")
    parser.add_argument("elf_path")
    parser.add_argument("readelf_path", help="the toolchain readelf; objdump is found beside it")
    parser.add_argument("nm_path", help="accepted for the build scripts' command line; not run")
    parser.add_argument("--toc-only", action="store_true", help="apply the TOC patch and leave every rldicr in place")
    return parser.parse_args(list(argv))


def report(err: BaseException) -> None:
    print(f"patch_toc: error: {err}", file=sys.stderr)
    cause = err.__cause__
    while cause is not None:
        print(f"patch_toc:   caused by: {type(cause).__name__}: {cause}", file=sys.stderr)
        cause = cause.__cause__


def main(argv: Sequence[str]) -> int:
    try:
        args = parse_args(argv)
        patch(args.elf_path, args.readelf_path, toc_only=args.toc_only)
    except PatchTocError as err:
        report(err)
        return err.exit_code
    except KeyboardInterrupt:
        print("patch_toc: interrupted", file=sys.stderr)
        return EXIT_INTERRUPTED
    except Exception as err:
        print(f"patch_toc: internal error: {type(err).__name__}: {err}", file=sys.stderr)
        if os.environ.get(TRACEBACK_ENV) == "1":
            traceback.print_exc()
        return EXIT_INTERNAL
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
