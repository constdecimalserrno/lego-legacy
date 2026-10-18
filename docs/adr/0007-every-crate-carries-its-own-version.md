# Every crate carries its own version, and a title's names everything it runs

A sign-off covers a title as it was on one commit. With one version shared by the whole
workspace and nothing tagged, nothing named that commit, nor which title changed since. So:
**each crate carries its own SemVer version in its own `Cargo.toml`, a title and the launcher pin
core exactly, each crate keeps a `CHANGELOG.md`, and a release tags every crate whose version
moved.** v1 is every crate at `1.0.0`.

A version is `major.minor.patch`, digits only, with no pre-release or build suffix.

## What each part of a version means

A **title**, any crate under `titles/`:

- **major** when its committed routes or its saves stop carrying over: a route no longer reaches
  the state it is named for, or a save the last version wrote is not read back.
- **minor** when it gains a reach: a milestone or a route.
- **patch** otherwise. Taking a core bump that moves none of its routes is a patch.

**Core**:

- **major** when a title has to change to keep building or to keep its routes.
- **minor** for something new a title can use.
- **patch** otherwise.

The **launcher** is ordinary SemVer for a command-line tool: major when a command line it took is
refused or does something else, minor when it takes something new, patch otherwise.

## A title's version names everything it runs

A title and the launcher depend on core with an exact requirement:

```toml
lego-legacy-core = { path = "../../core", version = "=1.0.0" }
```

Cargo refuses to build a crate whose requirement the core beside it does not meet. So a core bump
reaches a title only through a commit that changes the title's pin, and that commit changes a
file under the title, so the title's version moves as well. A title's version names its own code,
its routes and the core it runs on. A run's log says both in its header, `version 1.0.0 core
1.0.0`, so a run directory names what produced it.

## Every version says what changed

Each crate's `CHANGELOG.md` has one heading per version, `## 1.0.0 — 2026-10-18`, newest first,
saying what changed. A title's entry for a version that was signed off records the sign-off: the
date, the commit, and that it was played by the maintainer.

`core/tests/versions.rs` holds every crate to this, and `scripts/ci.sh` runs it. A crate's
version has a heading in its CHANGELOG. When a `<crate>-v*` tag exists and any file under the
crate differs from the newest one, the crate's version is greater than that tag's. A version
moves once between releases, not once per commit: a crate already past its tag passes however
much more it changes, and its heading grows to say so. The test reads tags with `git`, and in a
tree that is not a git checkout it checks only the changelogs.

What lives outside every crate (the workspace manifest, `Cargo.lock`, the toolchain pin, scripts
and docs) belongs to no crate's version.

## Tags, and rolling back

A release tags every crate whose version moved as `<crate>-v<version>`, `lego-alpha-team-v1.0.0`
for one, and the workspace as `v<version>`. The workspace's version moves by the largest part any
crate's moved in that release: major if any crate's major did, minor if any crate's minor did,
patch otherwise. v1 is `v1.0.0`, with all five crates' `-v1.0.0` on the same commit. A tag is
never moved or deleted, and a tag under a crate's name that is not a version fails the test.

Rolling a title back is `git checkout <title>-v<version>`: that commit is the title as it was
released, with the core it pinned.

## Considered options

- **One version for the workspace**, as `0.1.0` was: a change to one title moved every title's
  version, and a sign-off could not say which title it covered; rejected.
- **A caret requirement on core** (`version = "1.0.0"`): a title would build against any 1.x core
  without its own version moving, so its version would stop naming what it runs; rejected.
