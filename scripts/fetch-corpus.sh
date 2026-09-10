#!/bin/sh
# Fetch the x86 conformance corpus.
#
# SingleStepTests' 80386 suite: per-encoding recorded state pairs from real
# silicon. About 1.1 GB, not ours to redistribute, so it is fetched rather than
# committed and `vendor/` is ignored.
set -eu
cd "$(dirname "$0")/.."

dest=vendor/singlesteptests/80386
if [ -d "$dest/v1_ex_real_mode" ]; then
  printf 'corpus: already at %s\n' "$dest"
  exit 0
fi

mkdir -p vendor/singlesteptests
git clone --depth 1 https://github.com/SingleStepTests/80386 "$dest"
printf 'corpus: fetched %s files\n' "$(ls "$dest/v1_ex_real_mode" | wc -l | tr -d ' ')"
