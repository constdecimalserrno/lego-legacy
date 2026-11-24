#!/bin/sh
# Cross-target release-library build gate for lego-legacy-core (ADR-0009).
#
# Builds Core's pure-Rust rlib across all six desktop targets with zero
# cross-linkers and zero external crates. Uses `cargo build --lib --release`
# rather than `cargo check` because `--emit=metadata` skips LLVM codegen and
# silently passes invalid inline/naked assembly mnemonics.
set -eu
cd "$(dirname "$0")/.."

say() { printf '\n=== %s\n' "$1"; }

TARGETS="
aarch64-apple-darwin
x86_64-apple-darwin
aarch64-unknown-linux-gnu
x86_64-unknown-linux-gnu
aarch64-pc-windows-msvc
x86_64-pc-windows-msvc
"

for target in $TARGETS; do
    say "core --lib --release ($target)"
    cargo build -p lego-legacy-core --lib --release --target "$target"
done

printf '\ncheck-targets: green (all 6 desktop targets)\n'
