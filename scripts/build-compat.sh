#!/usr/bin/env bash
# Build Focuser for CPUs below x86-64-v3 (no FMA/AVX2) and for a glibc older
# than the one the upstream release was built against. See ../COMPAT.md.
set -euo pipefail

ROOT=$(cd "$(dirname "$0")/.." && pwd)
TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/target}
HOST=$(rustc -vV | awk '/^host:/ {print $2}')
cd "$ROOT"

# The toolchain's default baseline is what ends up in the bundled SQLite (cc-rs
# forwards the target features), so a toolchain that emits v3 would produce a
# binary that SIGILLs exactly like the upstream one.
if rustc --print cfg | grep -qE 'target_feature="(fma|avx2|avx512f)"'; then
    echo "erro: toolchain emitindo instruções acima do baseline x86-64" >&2
    echo "      rustflags ou target-cpu global agressivo; veja COMPAT.md" >&2
    exit 1
fi

echo "==> sidecars (focuser-cli, focuser-native) para $HOST"
CARGO_TARGET_DIR="$TARGET_DIR" cargo build --release --locked -p focuser-cli -p focuser-native

mkdir -p crates/focuser-ui/binaries
for bin in focuser-cli focuser-native; do
    cp "$TARGET_DIR/release/$bin" "crates/focuser-ui/binaries/$bin-$HOST"
done

echo "==> frontend"
( cd crates/focuser-ui/frontend && npm ci && npm run build )

echo "==> focuser-ui"
# `tauri/custom-protocol` is what makes the binary serve the embedded frontend.
# The Tauri CLI adds it itself; a bare `cargo build --release` does not, and the
# resulting app opens a window that says "Could not connect to localhost".
# TAURI_CONFIG drops externalBin: tauri-build 2.5 panics (remove_file on a
# directory) when the cargo target dir already holds build/<sidecar>/, which
# happens with cargo's newer build-dir layout. The sidecars are not needed at
# runtime (see COMPAT.md), so the local build simply does not bundle them.
TAURI_CONFIG='{"bundle":{"externalBin":[]}}' \
    CARGO_TARGET_DIR="$TARGET_DIR" \
    cargo build --release --locked -p focuser-ui --features tauri/custom-protocol

echo "==> verificação"
"$ROOT/scripts/verify-compat.sh" "$TARGET_DIR/release/focuser-ui"
for bin in focuser-cli focuser-native; do
    "$ROOT/scripts/verify-compat.sh" "$TARGET_DIR/release/$bin"
done
