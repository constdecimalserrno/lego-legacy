# Zero crates, and one host seam reached by dlopen

The emulator must still build and run twenty years from now, on macOS first and Linux and
Windows later. We decided: no external crates, ever (`Cargo.lock` resolves one package, and a test
holds it there), and every touch of the host, window, input, audio, wall clock, memory mapping,
dynamic loading, lives in one `host` module behind one trait, reaching the operating system's own
frameworks by `dlopen`/`dlsym` (and `objc_msgSend` on macOS). No third-party library is loaded,
not even SDL. Porting to a new OS is one new backend under that module and nothing else.

## Considered options

- **dlopen SDL3** (the previous tree): one Homebrew dylib away from not running; rejected.
- **Linking crates for windowing/audio**: rejected by the twenty-year rule.
