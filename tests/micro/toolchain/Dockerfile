# ps3dev + PSL1GHT toolchain image that builds tests/micro/*.
#
# Wraps the ps3dev umbrella project's prebuilt nightly (ps3toolchain +
# PSL1GHT + ps3libraries) instead of compiling gcc and newlib from
# source, so a build takes minutes. The nightly tag is pinned: every
# microtest ELF comes from the same compiler and PSL1GHT commit.
#
#   docker build -t ps3dev tests/micro/toolchain
#
# Bump the pin with
#   docker build --build-arg PS3DEV_NIGHTLY=nightly-YYYY-MM-DD -t ps3dev tests/micro/toolchain
# Tags are listed by `gh api repos/ps3dev/ps3dev/releases`.
ARG PS3DEV_NIGHTLY=nightly-2026-07-26

FROM ubuntu:24.04
ARG PS3DEV_NIGHTLY

ENV PS3DEV=/usr/local/ps3dev
ENV PSL1GHT=/usr/local/ps3dev
ENV PATH=${PATH}:/usr/local/ps3dev/bin:/usr/local/ps3dev/ppu/bin:/usr/local/ps3dev/spu/bin
ENV DEBIAN_FRONTEND=noninteractive

# Runtime deps of the prebuilt gcc/binutils (gmp/mpfr/mpc/elf/zlib), plus
# python3 for tests/micro/common/patch_toc.py.
RUN apt-get -y update && \
    apt-get -y install --no-install-recommends \
      ca-certificates curl tar xz-utils make python3 \
      libgmp10 libmpfr6 libmpc3 libelf1t64 zlib1g libncurses6 libssl3t64 && \
    apt-get -y clean && rm -rf /var/lib/apt/lists/*

RUN curl -fsSL --retry 3 \
      "https://github.com/ps3dev/ps3dev/releases/download/${PS3DEV_NIGHTLY}/ps3dev-linux-X64.tar.gz" \
      -o /tmp/ps3dev.tar.gz && \
    tar -xzf /tmp/ps3dev.tar.gz -C /usr/local && \
    rm /tmp/ps3dev.tar.gz && \
    echo "${PS3DEV_NIGHTLY}" > /usr/local/ps3dev/NIGHTLY_TAG && \
    powerpc64-ps3-elf-gcc --version | head -1 && \
    spu-gcc --version | head -1

WORKDIR /src
CMD ["bash"]
