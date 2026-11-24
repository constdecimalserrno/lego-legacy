//! ADR-0009: mechanical determinism guardrails for static host assembly and
//! SIMD in `core/src/`, plus end-to-end verification of `LEGO_LEGACY_ARCH`.

use lego_legacy_core::arch::{self, Tier};
use std::path::{Path, PathBuf};
use std::process::Command;

/// Strip Rust line (`//`) and nested block (`/* ... */`) comments while
/// keeping string literals intact so `asm!` / `naked_asm!` / `global_asm!`
/// templates remain visible to the scanner.
fn strip_comments(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    let mut block_depth = 0usize;
    let mut in_string = false;
    let mut raw_hashes: Option<usize> = None;

    while i < bytes.len() {
        if block_depth > 0 {
            if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
                block_depth += 1;
                i += 2;
            } else if i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
                block_depth -= 1;
                i += 2;
            } else {
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            continue;
        }

        if let Some(hashes) = raw_hashes {
            let ch = src[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            if ch == '"'
                && bytes.len() >= i + hashes
                && bytes[i..i + hashes].iter().all(|&b| b == b'#')
            {
                for _ in 0..hashes {
                    out.push('#');
                }
                i += hashes;
                raw_hashes = None;
            }
            continue;
        }

        if in_string {
            let ch = src[i..].chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
            if ch == '\\' && i < bytes.len() {
                let next = src[i..].chars().next().unwrap();
                out.push(next);
                i += next.len_utf8();
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        if i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            block_depth = 1;
            i += 2;
            continue;
        }

        // Raw string literal: r"..." or r#"..."# (or br"..." / cr"...").
        if bytes[i] == b'r' {
            let prev_is_ident =
                i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_');
            let allow_prefix = !prev_is_ident
                || (i >= 1
                    && (bytes[i - 1] == b'b' || bytes[i - 1] == b'c')
                    && (i == 1 || !(bytes[i - 2].is_ascii_alphanumeric() || bytes[i - 2] == b'_')));
            if allow_prefix {
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] == b'#' {
                    j += 1;
                }
                if j < bytes.len() && bytes[j] == b'"' {
                    let hashes = j - (i + 1);
                    out.push_str(&src[i..=j]);
                    i = j + 1;
                    raw_hashes = Some(hashes);
                    continue;
                }
            }
        }

        // Skip character literals like `'/'` or `'"'` without toggling string state.
        if bytes[i] == b'\'' {
            if i + 2 < bytes.len() && bytes[i + 1] != b'\\' && bytes[i + 2] == b'\'' {
                out.push_str(&src[i..i + 3]);
                i += 3;
                continue;
            }
            if i + 3 < bytes.len() && bytes[i + 1] == b'\\' && bytes[i + 3] == b'\'' {
                out.push_str(&src[i..i + 4]);
                i += 4;
                continue;
            }
        }

        let ch = src[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
        if ch == '"' {
            in_string = true;
        }
    }

    out
}

/// Split a line of comment-stripped source into lowercase identifier/mnemonic
/// tokens (`[a-z0-9_]+`).
fn tokens(line: &str) -> impl Iterator<Item = String> + '_ {
    line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|t| !t.is_empty())
        .map(str::to_ascii_lowercase)
}

fn forbidden_token_reason(tok: &str) -> Option<&'static str> {
    if tok == "mul_add"
        || matches!(
            tok,
            "fmadd" | "fmsub" | "fnmadd" | "fnmsub" | "fmla" | "fmls" | "fmlal" | "fmlsl"
        )
        || tok.starts_with("vfmadd")
        || tok.starts_with("vfmsub")
        || tok.starts_with("vfnmadd")
        || tok.starts_with("vfnmsub")
        || tok.starts_with("vfma_")
        || tok.starts_with("vfmaq_")
        || tok.starts_with("vfms_")
        || tok.starts_with("vfmsq_")
        || ((tok.starts_with("_mm_") || tok.starts_with("_mm256_") || tok.starts_with("_mm512_"))
            && (tok.contains("_fmadd")
                || tok.contains("_fmsub")
                || tok.contains("_fnmadd")
                || tok.contains("_fnmsub")))
    {
        return Some("FMA contraction");
    }

    if matches!(
        tok,
        "algebraic_add" | "algebraic_sub" | "algebraic_mul" | "algebraic_div" | "algebraic_rem"
    ) {
        return Some("algebraic_* float op");
    }

    if matches!(
        tok,
        "rcpss"
            | "rcpps"
            | "vrcpss"
            | "vrcpps"
            | "rsqrtss"
            | "rsqrtps"
            | "vrsqrtss"
            | "vrsqrtps"
            | "frecpe"
            | "frecps"
            | "frsqrte"
            | "frsqrts"
    ) || tok.starts_with("vrecpe_")
        || tok.starts_with("vrecpeq_")
        || tok.starts_with("vrecps_")
        || tok.starts_with("vrecpsq_")
        || tok.starts_with("vrsqrte_")
        || tok.starts_with("vrsqrteq_")
        || tok.starts_with("vrsqrts_")
        || tok.starts_with("vrsqrtsq_")
        || ((tok.starts_with("_mm_") || tok.starts_with("_mm256_") || tok.starts_with("_mm512_"))
            && (tok.contains("_rcp") || tok.contains("_rsqrt")))
    {
        return Some("approximate reciprocal/rsqrt");
    }

    if matches!(
        tok,
        "ldmxcsr"
            | "vldmxcsr"
            | "_mm_setcsr"
            | "_mm_set_rounding_mode"
            | "_mm_set_flush_zero_mode"
            | "_mm_set_denormals_zero_mode"
            | "fesetround"
    ) {
        return Some("host FP control-register mutation");
    }

    if matches!(tok, "x18" | "w18") {
        return Some("explicit aarch64 platform register x18/w18");
    }

    None
}

fn scan_source(label: &str, text: &str) -> Vec<String> {
    let stripped = strip_comments(text);
    let mut offenders = Vec::new();
    for (idx, line) in stripped.lines().enumerate() {
        let line_tokens: Vec<String> = tokens(line).collect();
        for tok in &line_tokens {
            if let Some(reason) = forbidden_token_reason(tok) {
                offenders.push(format!(
                    "{label}:{}: {reason} (`{tok}`): {}",
                    idx + 1,
                    line.trim()
                ));
            }
        }
        let has_msr = line_tokens.iter().any(|t| t == "msr");
        let has_fp_sysreg = line_tokens.iter().any(|t| t == "fpcr" || t == "fpsr");
        if has_msr && has_fp_sysreg {
            offenders.push(format!(
                "{label}:{}: host FP control-register mutation (`msr fpcr/fpsr`): {}",
                idx + 1,
                line.trim()
            ));
        }
    }
    offenders
}

fn rust_sources_under(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).expect("source directory reads") {
            let path = entry.expect("directory entry").path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    files
}

#[test]
fn core_source_has_no_determinism_breaking_arch_primitives() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = rust_sources_under(&src_root);
    assert!(
        files.iter().any(|p| p.ends_with("arch.rs")),
        "expected core/src/arch.rs in scanned tree"
    );

    let mut offenders = Vec::new();
    for path in files {
        let text = std::fs::read_to_string(&path).expect("source file reads");
        offenders.extend(scan_source(&path.display().to_string(), &text));
    }
    assert!(
        offenders.is_empty(),
        "ADR-0009 bans determinism-breaking primitives in core/src/:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn scanner_ignores_comments_and_rejects_every_forbidden_category() {
    let comments_only = r#"
        // mul_add, algebraic_add, _mm_rcp_ps, ldmxcsr, msr fpcr, x18
        /* nested /* vfmaq_f32 vfmadd213ps frecpe */ w18 */
        let ok = a * b + c;
        let slash = '/';
    "#;
    assert_eq!(scan_source("ok.rs", comments_only), Vec::<String>::new());

    let cases: &[(&str, &str)] = &[
        ("let v = a.mul_add(b, c);", "FMA contraction"),
        ("let v = _mm256_fmadd_ps(a, b, c);", "FMA contraction"),
        ("let v = vfmaq_f32(a, b, c);", "FMA contraction"),
        ("asm!(\"vfmadd213ps ymm0, ymm1, ymm2\");", "FMA contraction"),
        ("asm!(\"fmla v0.4s, v1.4s, v2.4s\");", "FMA contraction"),
        ("let v = a.algebraic_add(b);", "algebraic_* float op"),
        ("let v = f32::algebraic_mul(a, b);", "algebraic_* float op"),
        ("let v = _mm_rcp_ps(x);", "approximate reciprocal/rsqrt"),
        ("let v = _mm_rsqrt_ps(x);", "approximate reciprocal/rsqrt"),
        ("let v = vrecpeq_f32(x);", "approximate reciprocal/rsqrt"),
        ("let v = vrsqrteq_f32(x);", "approximate reciprocal/rsqrt"),
        (
            "asm!(\"rcpps xmm0, xmm1\");",
            "approximate reciprocal/rsqrt",
        ),
        (
            "asm!(\"frecpe v0.4s, v1.4s\");",
            "approximate reciprocal/rsqrt",
        ),
        (
            "asm!(\"ldmxcsr [rax]\");",
            "host FP control-register mutation",
        ),
        ("_mm_setcsr(bits);", "host FP control-register mutation"),
        (
            "fesetround(FE_TOWARDZERO);",
            "host FP control-register mutation",
        ),
        (
            "asm!(\"MSR FPCR, x0\");",
            "host FP control-register mutation",
        ),
        (
            "asm!(\"msr fpsr, x0\");",
            "host FP control-register mutation",
        ),
        (
            "asm!(\"mov x0, x1\", in(\"x18\") val);",
            "explicit aarch64 platform register x18/w18",
        ),
        (
            "naked_asm!(\"mov x18, x0\");",
            "explicit aarch64 platform register x18/w18",
        ),
        (
            "naked_asm!(\"str w18, [sp]\");",
            "explicit aarch64 platform register x18/w18",
        ),
    ];

    for (snippet, expected_reason) in cases {
        let hits = scan_source("injected.rs", snippet);
        assert!(
            hits.iter().any(|h| h.contains(expected_reason)),
            "expected `{expected_reason}` for `{snippet}`, got {hits:?}"
        );
    }
}

#[test]
fn lego_legacy_arch_is_not_referenced_by_guest_state_or_shims() {
    let src_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut offenders = Vec::new();
    for path in rust_sources_under(&src_root) {
        if path.ends_with("arch.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("source file reads");
        let stripped = strip_comments(&text);
        if stripped.contains("LEGO_LEGACY_ARCH") {
            offenders.push(path.display().to_string());
        }
    }
    assert!(
        offenders.is_empty(),
        "only core/src/arch.rs may read LEGO_LEGACY_ARCH, found in: {offenders:?}"
    );
}

#[test]
fn startup_env_override_controls_active_tier_and_is_cached_once() {
    if std::env::var_os("LEGO_LEGACY_ARCH_TEST_CHILD").is_some() {
        let expected = std::env::var("LEGO_LEGACY_ARCH_TEST_EXPECT").expect("expected tier");
        match expected.as_str() {
            "rust" => {
                assert_eq!(Tier::active(), Tier::Rust);
                assert!(arch::use_rust());
                std::env::set_var(arch::ENV_VAR, "native");
                assert_eq!(Tier::active(), Tier::Rust);
                assert!(arch::use_rust());
            }
            "native" => {
                assert_eq!(Tier::active(), Tier::native());
                assert_eq!(arch::use_rust(), Tier::native() == Tier::Rust);
                std::env::set_var(arch::ENV_VAR, "rust");
                assert_eq!(Tier::active(), Tier::native());
            }
            "panic" => {
                let _ = Tier::active();
            }
            other => panic!("unexpected child mode {other}"),
        }
        return;
    }

    let exe = std::env::current_exe().expect("test executable path");
    let run_child = |arch_val: Option<&str>, expect: &str| {
        let mut cmd = Command::new(&exe);
        cmd.args([
            "--exact",
            "startup_env_override_controls_active_tier_and_is_cached_once",
            "--test-threads=1",
        ])
        .env("LEGO_LEGACY_ARCH_TEST_CHILD", "1")
        .env("LEGO_LEGACY_ARCH_TEST_EXPECT", expect)
        .env_remove(arch::ENV_VAR);
        if let Some(v) = arch_val {
            cmd.env(arch::ENV_VAR, v);
        }
        cmd.output().expect("spawn child test")
    };

    let out_rust = run_child(Some("rust"), "rust");
    assert!(
        out_rust.status.success(),
        "LEGO_LEGACY_ARCH=rust failed:\n{}\n{}",
        String::from_utf8_lossy(&out_rust.stdout),
        String::from_utf8_lossy(&out_rust.stderr)
    );

    let out_default = run_child(None, "native");
    assert!(
        out_default.status.success(),
        "unset LEGO_LEGACY_ARCH failed:\n{}\n{}",
        String::from_utf8_lossy(&out_default.stdout),
        String::from_utf8_lossy(&out_default.stderr)
    );

    let out_native = run_child(Some("native"), "native");
    assert!(
        out_native.status.success(),
        "LEGO_LEGACY_ARCH=native failed:\n{}\n{}",
        String::from_utf8_lossy(&out_native.stdout),
        String::from_utf8_lossy(&out_native.stderr)
    );

    let out_invalid = run_child(Some("bogus"), "panic");
    assert!(
        !out_invalid.status.success(),
        "invalid LEGO_LEGACY_ARCH must fail loudly"
    );
}
