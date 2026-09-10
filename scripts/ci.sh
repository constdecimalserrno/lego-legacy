#!/bin/sh
# The one definition of correct for this repository.
#
# Everything a change must pass, in the order that fails fastest. No lane has a
# second definition: if it is not checked here, it is not checked.
set -eu
cd "$(dirname "$0")/.."

say() { printf '\n=== %s\n' "$1"; }

say 'formatting'
cargo fmt --all -- --check

say 'clippy'
cargo clippy --workspace --all-targets -- -D warnings

say 'tests'
# Runs land in the target scratch root, never in the working tree.
LEGO_LEGACY_RUNS="${CARGO_TARGET_DIR:-target}/runs" cargo test --workspace

say 'release build'
cargo build --workspace --release

printf '\nci: green\n'
