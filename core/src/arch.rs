//! Host-side architecture dispatch plumbing (ADR-0009).
//!
//! Accelerated subsystems keep their `aarch64`, `x86_64`, and canonical
//! pure-Rust implementations co-located beside their own invariants. This
//! module holds only the startup override (`LEGO_LEGACY_ARCH`) and the host
//! tier selection shared across those seams.
//!
//! Nothing here is guest-visible, read by any shim, or hashed into a machine
//! digest (Golden Rule 1): every tier produces results bit-for-bit identical
//! to [`Tier::Rust`].

use std::sync::OnceLock;

/// Startup environment variable that overrides native host kernel dispatch.
///
/// Set `LEGO_LEGACY_ARCH=rust` to force every accelerated subsystem onto its
/// canonical pure-Rust path without recompiling.
pub const ENV_VAR: &str = "LEGO_LEGACY_ARCH";

/// Host execution tier selected for an accelerated subsystem.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// Canonical pure-Rust implementation (oracle and universal fallback).
    Rust,
    /// Static `aarch64` host implementation (baseline NEON / `aarch64` asm).
    Aarch64,
    /// Static `x86_64` host implementation (baseline SSE2 / `x86_64` asm).
    X86_64,
}

impl Tier {
    /// The compile-time tier for this host target.
    #[must_use]
    pub const fn native() -> Self {
        #[cfg(target_arch = "aarch64")]
        {
            Self::Aarch64
        }
        #[cfg(target_arch = "x86_64")]
        {
            Self::X86_64
        }
        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        {
            Self::Rust
        }
    }

    /// Process-wide active tier, resolved once from `LEGO_LEGACY_ARCH`.
    ///
    /// Panics if `LEGO_LEGACY_ARCH` is set to an unrecognized or non-UTF-8
    /// value so a typo never silently runs the wrong tier.
    #[must_use]
    pub fn active() -> Self {
        static ACTIVE: OnceLock<Tier> = OnceLock::new();
        *ACTIVE.get_or_init(|| {
            let value = std::env::var_os(ENV_VAR).map(|raw| {
                raw.into_string().unwrap_or_else(|_| {
                    panic!("invalid {ENV_VAR} (non-UTF-8): expected \"rust\" or \"native\"")
                })
            });
            Self::resolve(value.as_deref(), Self::native()).unwrap_or_else(|err| panic!("{err}"))
        })
    }

    /// Resolve a tier from an optional `LEGO_LEGACY_ARCH` value and the
    /// host's compile-time native tier.
    pub fn resolve(override_value: Option<&str>, native: Self) -> Result<Self, String> {
        let Some(raw) = override_value else {
            return Ok(native);
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("native") {
            Ok(native)
        } else if trimmed.eq_ignore_ascii_case("rust") {
            Ok(Self::Rust)
        } else {
            Err(format!(
                "invalid {ENV_VAR}={raw:?}: expected \"rust\" or \"native\""
            ))
        }
    }
}

/// Whether accelerated subsystems must execute their canonical pure-Rust path.
#[inline]
#[must_use]
pub fn use_rust() -> bool {
    Tier::active() == Tier::Rust
}

#[cfg(test)]
mod tests {
    use super::Tier;

    #[test]
    fn native_matches_compile_time_target_arch() {
        #[cfg(target_arch = "aarch64")]
        assert_eq!(Tier::native(), Tier::Aarch64);

        #[cfg(target_arch = "x86_64")]
        assert_eq!(Tier::native(), Tier::X86_64);

        #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64")))]
        assert_eq!(Tier::native(), Tier::Rust);
    }

    #[test]
    fn resolve_defaults_to_native_when_unset_empty_or_native() {
        for native in [Tier::Rust, Tier::Aarch64, Tier::X86_64] {
            assert_eq!(Tier::resolve(None, native), Ok(native));
            assert_eq!(Tier::resolve(Some(""), native), Ok(native));
            assert_eq!(Tier::resolve(Some("   "), native), Ok(native));
            assert_eq!(Tier::resolve(Some("native"), native), Ok(native));
            assert_eq!(Tier::resolve(Some("NATIVE"), native), Ok(native));
            assert_eq!(Tier::resolve(Some(" native "), native), Ok(native));
        }
    }

    #[test]
    fn resolve_forces_pure_rust_on_rust_override() {
        for native in [Tier::Rust, Tier::Aarch64, Tier::X86_64] {
            assert_eq!(Tier::resolve(Some("rust"), native), Ok(Tier::Rust));
            assert_eq!(Tier::resolve(Some("RUST"), native), Ok(Tier::Rust));
            assert_eq!(Tier::resolve(Some(" Rust "), native), Ok(Tier::Rust));
        }
    }

    #[test]
    fn resolve_rejects_unknown_override_values() {
        for bad in ["ruts", "sse2", "neon", "avx2", "1"] {
            let err = Tier::resolve(Some(bad), Tier::Aarch64)
                .expect_err("unknown LEGO_LEGACY_ARCH must be refused");
            assert!(err.contains("LEGO_LEGACY_ARCH"), "{err}");
            assert!(err.contains(bad), "{err}");
        }
    }
}
