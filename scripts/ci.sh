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

# The suite runs optimised, because the disc tests retire a hundred million
# guest instructions and unpack eighty megabytes on the way, which is half a
# minute unoptimised and three seconds optimised.
say 'tests'
# Runs land in the target scratch root, never in the working tree.
LEGO_LEGACY_RUNS="${CARGO_TARGET_DIR:-target}/runs" cargo test --workspace --release

# The unit tests run again unoptimised, where arithmetic that overflows panics
# instead of wrapping. That is how the heap's own rounding bug was found, and
# it costs a second.
say 'tests, with overflow checks'
LEGO_LEGACY_RUNS="${CARGO_TARGET_DIR:-target}/runs" cargo test -p lego-legacy-core --lib

printf '\nci: green\n'
