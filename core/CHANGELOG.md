# Changelog: lego-legacy-core

One heading per version, newest first. What each part of core's version means is in
[ADR-0007](../docs/adr/0007-every-crate-carries-its-own-version.md).

## 1.1.0 — 2026-10-18

The Windows backend joined (#107): a window through user32 and gdi32, the pointer by macOS's rules, and sound through winmm's `waveOut`, loaded from System32 at run time, with kernel32's `VirtualAlloc` and `LoadLibraryExW` beside `mmap` and `dlopen`. It is written and compiles for x86-64 and Arm64 Windows, and is untested on Windows.

## 1.0.0 — 2026-10-18

The engine the three v1 titles run on: an x86 and x87 interpreter for the guest's code, the Win32
and DirectX calls those titles make answered in Rust, a declared machine per title that the title
cannot see past, and a macOS host reached at run time with no external crates (ADR-0001). A run is
replayed from a route, logs what it did, and stops at a named wall when a title asks for something
the machine does not have.
