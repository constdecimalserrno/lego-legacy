# Plan

A rebuild of `/Volumes/Tabletop/Lego/old/lego-legacy`, from an empty `src/`. The old tree is
reference only; no code, ADRs or goldens come across. Vocabulary is in `CONTEXT.md`, load-bearing
decisions in `docs/adr/`.

## Goal

Four titles reach **complete** (every unit of play completable from the keyboard, saves included),
with audio, video and working controls, at pace 1.0× and presentation rate ≥ intended rate on the
dev mac. **Credits** is the goal beyond that, reached by the maintainer playing.

## Roster, in order

1. `lego-rock-raiders` — original pressing (the only pressing with Red Book audio; not SafeDisc)
2. `hotwheels-stunt-track-driver` — original pressing
3. `lego-alpha-team` — original pressing
4. `lego-racers` — Masterpiece pressing (the original is SafeDisc: named and refused, never bypassed)

Exactly four. A second pressing is supported only when its entry image runs unwrapped. A title may
declare another pressing as its Red Book source; today none needs to.

## Constraints

- Zero external crates, forever. A test holds `Cargo.lock` to one package. (ADR-0001)
- macOS first; Linux and Windows are one new backend under `core/src/host/` and nothing else.
- No JIT. No native override of guest code. The machine runs the title's bytes or stops. (ADR-0004)
- Deterministic: the input script is the only recorded input. (ADR-0003)
- Audio off by default; mixing always runs, only the host device is gated.
- Every title's window resizes; the guest never sees the window size.

## Architecture

**Workspace**: `core/` (lib), `launcher/` (bin `lego-legacy`), `titles/<roster-key>/` (one bin
each, named by roster key). Per-title declared values and title-only shims live in the title crate.
Anything two titles need moves to core on the second use.

**Host seam** (ADR-0001): one `Host` trait in core covering window, input, audio sink, wall clock,
memory mapping and dynamic loading. macOS backend reaches AppKit, CoreGraphics and AudioToolbox by
`dlopen`/`dlsym`/`objc_msgSend`. No SDL.

**Boundary** (ADR-0002): the title's own executable and DLLs run as guest x86. Microsoft's and
vendor DLLs (KERNEL32, USER32, MSVCRT, DDRAW, D3DIM, D3DRM, DSOUND, DINPUT, A3D) are Rust shims at
their export tables. A DLL shipped on the disc may run instead when that is cheaper. An import no
shim implements is a **wall**: the run stops and names the import, caller and cursor. Every chosen
return value is a declared value with its reason beside it.

**CPU** (ADR-0004): x86-32 decode-cached interpreter (x87, MMX as titles need them) behind a
`CpuEngine` seam. Conformance against the SingleStepTests 80386 corpus, fetched by script into a
gitignored `vendor/`.

**Declared machine**: Win98 SE, Pentium II 450, 128 MB, TNT2-class DX6/7 caps, 640×480 modes as
the baseline, each value carrying its reason and re-measured per title. **Timebase** (ADR-0003):
the declared clock derives from instructions retired plus declared costs (a flip, an audio block);
no host timebase. The host throttles flips to pace 1.0×; headless runs are uncapped. **Threads**
(ADR-0005): cooperative, one host thread, switch at a declared quantum and at blocking waits.

**Video**: software rasteriser into guest-visible surface bytes; the host scales the final frame,
aspect-locked, any size. Multithreading only through a declared decomposition, only after a
measurement names the rasteriser as the wall. AVI/Cinepak/Indeo/CRAM/ADPCM decoders in core.
Movie skipping is the guest's own business.

**Audio**: DSound and A3D shims mix into one machine-clocked stream; the host device consumes it.
Red Book tracks read from the pressing's bin/cue.

**Input**: keyboard and mouse. Gamepad when a title needs it.

**Data**: rips stay pristine in `game-data/<title>/<pressing>/` (gitignored). The declared install
tree and writable profile (saves) live at the OS data dir, `~/Library/Application Support/lego-legacy/<title>/`
on mac, `--data-root` to override, built lazily by the title binary on first run.

**Launcher**: no args lists detected pressings and prompts; `lego-legacy <title> [flags]` execs
the title binary, forwarding flags. Flags are identical across titles via core's CLI.

## Determinism, routes, debugging

- **Route**: one line-oriented text format keyed by flip number, `#` comments. The recorder writes
  it, a human can author it, replay delivers at the flip boundary.
- **Run log**: every run gets a gitignored `runs/<id>/` with inputs always recorded; shim-call and
  instruction trace behind `--trace`; a crash always dumps inputs + cursor + last snapshot.
- **Snapshots**: off by default; `--snapshot-every <flips>`; a cache, never committed; resume
  refuses another build's snapshot.
- **Debugger**: a stdin REPL in every title binary (`step`, `regs`, `mem`, `break`, `until`,
  `continue`, `snapshot`) plus resume-at-cursor from a snapshot.

## Evidence

**Until all four run, no goldens.** They slow development and are the last thing built. `cargo test`
holds three cheap gates instead:

1. the x86 conformance corpus;
2. routes that must reach a named guest state (a file written, a flip count) within an instruction
   budget, no hashes;
3. a determinism gate: replay one route twice, require identical final machine state.

**Goldens, last**: per route, hash every presented frame, every audio block, and the guest's written
files at route end; pixels and WAVs never commit; 10 minutes wall per title total, headless,
enforced by the test; a manifest-equality test refuses any hash on disk no test re-derives.
Blessing stays the maintainer's act.

## Milestones per title

first light → playable → complete → credits. Unit of play: a mission (Rock Raiders, Alpha Team), a
race (Hot Wheels, Racers). *Playable* needs a replaying route and a person saying so. *Complete* is
verified by the maintainer's recorded sessions, replayable but unhashed until goldens land.

## Hygiene

`rust-toolchain.toml` pins stable. rustfmt and clippy clean from commit one. `cargo test` is CI.
ADRs numbered sequentially at write time. Issues on GitHub per `docs/agents/issue-tracker.md`.

## First ticket set (Rock Raiders to first light)

1. Workspace skeleton, toolchain pin, zero-crate test, CI script.
2. `Host` trait + macOS backend: window, present a framebuffer, keyboard/mouse events, wall clock.
3. Guest memory arena + PE loader for `LegoRR.exe` and its shipped DLLs.
4. x86 decoder/interpreter to the conformance corpus.
5. Shim registry, wall reporting, declared-value carrier, run log.
6. Disc: ISO-9660 reader over the rip, declared install tree, data root, profile.
7. Rock Raiders title crate: declared machine, KERNEL32/USER32/MSVCRT shims to first light.
8. DDraw + D3DRM shims and the rasteriser, to Rock Raiders first light.
9. Route format, recorder, replay, determinism gate.
10. Launcher binary.
