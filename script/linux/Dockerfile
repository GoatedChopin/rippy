# Builds Rippy for Linux on Ubuntu 22.04, so the binary needs glibc 2.35 or newer: Ubuntu 22.04,
# Debian 12, Fedora 36 and later. script/release-linux.sh runs it. The Rust toolchain comes
# from the official Rust image.
FROM rust:1.95-bookworm AS rust

FROM ubuntu:22.04
COPY --from=rust /usr/local/rustup /usr/local/rustup
COPY --from=rust /usr/local/cargo /usr/local/cargo
ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
      build-essential ca-certificates clang cmake perl pkg-config zip \
      libfontconfig1-dev libfreetype6-dev libwayland-dev libx11-xcb-dev libxcb1-dev \
      libxkbcommon-dev libxkbcommon-x11-dev \
 && rm -rf /var/lib/apt/lists/*
