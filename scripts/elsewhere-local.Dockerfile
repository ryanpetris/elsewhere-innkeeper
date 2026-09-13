# Node.js 24 on Debian 12.
FROM node:24-slim@sha256:2fe369e969550cde8e867afc3fe370b260140cab4a23d467074295b42163d553 AS node

# Rust 1 on Debian 13.
FROM rust:1@sha256:bf5a9aa29062a6cb03c49bd59a46eb55e3cc770caf598a221a7866e500be3082 AS debian-build
RUN apt-get update && apt-get install -y --no-install-recommends \
    make git pkg-config cmake libavcodec-dev libavutil-dev libavfilter-dev \
    libswscale-dev libswresample-dev libavformat-dev libavdevice-dev libva-dev \
    libpipewire-0.3-dev libclang-dev libgbm-dev libegl-dev libxkbcommon-dev libglib2.0-bin \
    && rm -rf /var/lib/apt/lists/*
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
    && cargo install cargo-deb --locked
RUN useradd -m builder && mkdir /out && chown builder:builder /out
USER builder
ENV HOME=/home/builder CARGO_HOME=/home/builder/.cargo
WORKDIR /home/builder/source
COPY --chown=builder:builder . .
ARG ELSEWHERE_VERSION
ENV ELSEWHERE_VERSION=$ELSEWHERE_VERSION
# Packaging uses the version derived from the host checkout, including dirty edits.
RUN test -n "$ELSEWHERE_VERSION" \
    && printf '%s\n' '#!/bin/sh' 'printf "%s\n" "$ELSEWHERE_VERSION"' > scripts/version.sh \
    && make package-deb \
    && version=$(printf '%s' "${ELSEWHERE_VERSION#v}" | tr '-' '.') \
    && package="dist/elsewhere_${version}-1_debian-13_amd64.deb" \
    && test "$(dpkg-deb -f "$package" Package)" = elsewhere \
    && test "$(dpkg-deb -f "$package" Version)" = "$version-1" \
    && test "$(dpkg-deb -f "$package" Architecture)" = amd64 \
    && cp "$package" /out/

# Rust 1 on Debian 12.
FROM rust@sha256:9a73a5088750b4c95158ab26629c854c3d6fc4b173cb7bc8079ad252d8ed7bfa AS ubuntu-rust
FROM ubuntu:26.04 AS ubuntu-build
COPY --from=ubuntu-rust /usr/local/cargo /usr/local/cargo
COPY --from=ubuntu-rust /usr/local/rustup /usr/local/rustup
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN apt-get update && apt-get install -y --no-install-recommends \
    build-essential curl ca-certificates make git pkg-config cmake libavcodec-dev libavutil-dev libavfilter-dev \
    libswscale-dev libswresample-dev libavformat-dev libavdevice-dev libva-dev \
    libpipewire-0.3-dev libclang-dev libgbm-dev libegl-dev libxkbcommon-dev libglib2.0-bin \
    && rm -rf /var/lib/apt/lists/*
COPY --from=node /usr/local/bin/node /usr/local/bin/node
COPY --from=node /usr/local/lib/node_modules /usr/local/lib/node_modules
RUN ln -s ../lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s ../lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
    && cargo install cargo-deb --locked
RUN useradd -m builder && mkdir /out && chown builder:builder /out
USER builder
ENV HOME=/home/builder CARGO_HOME=/home/builder/.cargo
WORKDIR /home/builder/source
COPY --chown=builder:builder . .
ARG ELSEWHERE_VERSION
ENV ELSEWHERE_VERSION=$ELSEWHERE_VERSION
# Packaging uses the version derived from the host checkout, including dirty edits.
RUN test -n "$ELSEWHERE_VERSION" \
    && printf '%s\n' '#!/bin/sh' 'printf "%s\n" "$ELSEWHERE_VERSION"' > scripts/version.sh \
    && make package-deb \
    && version=$(printf '%s' "${ELSEWHERE_VERSION#v}" | tr '-' '.') \
    && package="dist/elsewhere_${version}-1_ubuntu-26.04_amd64.deb" \
    && test "$(dpkg-deb -f "$package" Package)" = elsewhere \
    && test "$(dpkg-deb -f "$package" Version)" = "$version-1" \
    && test "$(dpkg-deb -f "$package" Architecture)" = amd64 \
    && cp "$package" /out/

FROM archlinux:base-devel AS arch-build
RUN pacman-key --init && pacman-key --populate archlinux \
    && pacman -Sy --noconfirm archlinux-keyring \
    && pacman -Syu --noconfirm --needed cargo nodejs npm ffmpeg libva \
    mesa libxkbcommon libpipewire clang glib2 git \
    && rm -rf /var/cache/pacman/pkg/*
RUN useradd -m builder && mkdir /out && chown builder:builder /out
USER builder
ENV HOME=/home/builder CARGO_HOME=/home/builder/.cargo
WORKDIR /home/builder/source
COPY --chown=builder:builder . .
ARG ELSEWHERE_VERSION
ENV ELSEWHERE_VERSION=$ELSEWHERE_VERSION
# Packaging uses the version derived from the host checkout, including dirty edits.
RUN test -n "$ELSEWHERE_VERSION" \
    && printf '%s\n' '#!/bin/sh' 'printf "%s\n" "$ELSEWHERE_VERSION"' > scripts/version.sh \
    && make package-arch \
    && version=$(printf '%s' "${ELSEWHERE_VERSION#v}" | tr '-' '.') \
    && package="dist/elsewhere-${version}-1-x86_64.pkg.tar.zst" \
    && bsdtar -xOf "$package" .PKGINFO > /tmp/pkginfo \
    && grep -Fx 'pkgname = elsewhere' /tmp/pkginfo \
    && grep -Fx "pkgver = $version-1" /tmp/pkginfo \
    && grep -Fx 'arch = x86_64' /tmp/pkginfo \
    && cp "$package" /out/

FROM scratch AS debian
COPY --from=debian-build /out/ /

FROM scratch AS arch
COPY --from=arch-build /out/ /

FROM scratch AS ubuntu
COPY --from=ubuntu-build /out/ /
