#!/bin/sh
# The one definition of correct for this repository.
#
# Everything a change must pass, in the order that fails fastest. No lane has a
# second definition: if it is not checked here, it is not checked.
#
# On Windows this runs under Git Bash (`sh scripts/ci.sh`), which Git for
# Windows installs beside the `git` the suite needs anyway (core/tests/
# versions.rs). A PowerShell copy of these steps would be a second definition
# of correct, which is the one thing this script exists not to have (#107).
set -eu
cd "$(dirname "$0")/.."

say() { printf '\n=== %s\n' "$1"; }

say 'formatting'
cargo fmt --all -- --check

say 'clippy'
cargo clippy --workspace --all-targets -- -D warnings

# The suite runs optimised, because the disc tests replay every committed
# route, Alpha Team's training alone over twenty-three billion guest
# instructions. That is about ten minutes optimised; unoptimised the
# interpreter is twenty times slower, which is hours.
say 'tests'
# Runs land under the target directory, never among the sources.
LEGO_LEGACY_RUNS="${CARGO_TARGET_DIR:-target}/runs" cargo test --workspace --release

# The unit tests run again unoptimised, where arithmetic that overflows panics
# instead of wrapping. That is how the heap's own rounding bug was found, and
# the run costs two seconds.
say 'tests, with overflow checks'
LEGO_LEGACY_RUNS="${CARGO_TARGET_DIR:-target}/runs" cargo test -p lego-legacy-core --lib

printf '\nci: green\n'
