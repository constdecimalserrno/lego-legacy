#!/bin/sh
# Fetch the x86 conformance corpus.
#
# SingleStepTests' 80386 suite: per-encoding recorded state pairs from real
# silicon. About 1.1 GB, so it is fetched rather than committed and `vendor/` is
# ignored. Pinned to the commit the suite was last run green against.
set -eu
cd "$(dirname "$0")/.."

dest=vendor/singlesteptests/80386
if [ -d "$dest/v1_ex_real_mode" ]; then
  printf 'corpus: already at %s\n' "$dest"
  exit 0
fi

rev=655b1e947b5ce52d99fb7538095a7eb78874e89f
git init -q "$dest"
git -C "$dest" fetch -q --depth 1 https://github.com/SingleStepTests/80386 "$rev"
git -C "$dest" checkout -q FETCH_HEAD
printf 'corpus: fetched %s files\n' "$(ls "$dest/v1_ex_real_mode" | wc -l | tr -d ' ')"
