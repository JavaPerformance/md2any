#!/usr/bin/env bash
# Build md2any for all five supported targets and stage the binaries
# under dist/ with descriptive names.
#
# Targets:
#   x86_64-unknown-linux-gnu    Linux x86_64, dynamic glibc (smallest binary, needs glibc on host)
#   x86_64-unknown-linux-musl   Linux x86_64, static-PIE (runs on any Linux kernel, no glibc dep)
#   x86_64-pc-windows-gnu       Windows x86_64 (.exe)
#   x86_64-apple-darwin         macOS Intel
#   aarch64-apple-darwin        macOS Apple Silicon (M1/M2/M3/M4)
#
# Toolchain prerequisites (one-time setup):
#   rustup target add x86_64-unknown-linux-musl
#   rustup target add x86_64-pc-windows-gnu
#   rustup target add x86_64-apple-darwin aarch64-apple-darwin
#   cargo install cargo-zigbuild
#   <package manager> install zig
#
# Why cargo-zigbuild for Windows + macOS?
#   - Windows GNU needs a working mingw-w64 with libgcc_eh; zig sidesteps that.
#   - macOS targets need a macOS SDK; zig ships an SDK stub that satisfies the linker.
#   - Result: cross-compile to Windows + both Mac architectures from Linux with no
#     proprietary tools, no Docker, no Xcode.
#
# Usage:
#   ./scripts/build-all.sh            build all five targets
#   ./scripts/build-all.sh --linux    build only the two Linux targets
#   ./scripts/build-all.sh --windows  build only Windows
#   ./scripts/build-all.sh --macos    build both macOS targets

set -euo pipefail

cd "$(dirname "$0")/.."

DIST="${DIST_DIR:-dist}"
mkdir -p "$DIST"

build_linux_gnu() {
    echo "==> x86_64-unknown-linux-gnu"
    cargo build --release --target x86_64-unknown-linux-gnu
    cp "target/x86_64-unknown-linux-gnu/release/md2any" "$DIST/md2any-linux-x86_64-gnu"
}

build_linux_musl() {
    echo "==> x86_64-unknown-linux-musl"
    cargo build --release --target x86_64-unknown-linux-musl
    cp "target/x86_64-unknown-linux-musl/release/md2any" "$DIST/md2any-linux-x86_64-musl"
}

build_windows() {
    echo "==> x86_64-pc-windows-gnu"
    cargo zigbuild --release --target x86_64-pc-windows-gnu
    cp "target/x86_64-pc-windows-gnu/release/md2any.exe" "$DIST/md2any-windows-x86_64.exe"
}

build_macos_x86() {
    echo "==> x86_64-apple-darwin (Intel Mac)"
    cargo zigbuild --release --target x86_64-apple-darwin
    cp "target/x86_64-apple-darwin/release/md2any" "$DIST/md2any-macos-x86_64"
}

build_macos_arm() {
    echo "==> aarch64-apple-darwin (Apple Silicon)"
    cargo zigbuild --release --target aarch64-apple-darwin
    cp "target/aarch64-apple-darwin/release/md2any" "$DIST/md2any-macos-aarch64"
}

case "${1:-all}" in
    --linux)   build_linux_gnu; build_linux_musl ;;
    --windows) build_windows ;;
    --macos)   build_macos_x86; build_macos_arm ;;
    all|--all|"")
        build_linux_gnu
        build_linux_musl
        build_windows
        build_macos_x86
        build_macos_arm
        ;;
    *)
        echo "unknown option: $1" >&2
        echo "usage: $0 [--linux | --windows | --macos | --all]" >&2
        exit 1
        ;;
esac

echo
echo "Built binaries in $DIST/:"
ls -lh "$DIST"/md2any-* 2>/dev/null || true
