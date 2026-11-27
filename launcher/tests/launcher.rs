//! Integration tests for the `lego-legacy` launcher binary (#389).

use std::path::PathBuf;
use std::process::{Command, Stdio};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_lego-legacy"))
}

fn empty_dir(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("launcher-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

#[cfg(unix)]
fn install_stub_dir(name: &str, keys: &[&str]) -> (PathBuf, PathBuf) {
    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;

    let dir = empty_dir(name);
    let launcher_copy = dir.join("lego-legacy");
    std::fs::copy(bin(), &launcher_copy).expect("copy launcher binary");

    for key in keys {
        let stub = dir.join(key);
        let script = format!("#!/bin/sh\nprintf 'BIN=%s ARGS=%s\\n' '{key}' \"$*\"\n");
        std::fs::write(&stub, script).expect("write stub");
        std::fs::set_permissions(&stub, Permissions::from_mode(0o755)).expect("chmod stub");
    }
    (dir, launcher_copy)
}

#[test]
fn help_exits_zero_and_documents_usage() {
    for flag in ["--help", "-h"] {
        let out = Command::new(bin())
            .arg(flag)
            .output()
            .expect("run launcher --help");
        assert!(out.status.success(), "{flag} failed: {:?}", out.status);
        let stdout = String::from_utf8_lossy(&out.stdout);
        assert!(stdout.contains("usage: lego-legacy"), "stdout:\n{stdout}");
        assert!(
            stdout.contains("short") || stdout.contains("racers"),
            "help should mention short title names:\n{stdout}"
        );
    }
}

#[test]
fn no_args_renders_plain_ascii_banner_and_roster_only_when_piped() {
    let empty = empty_dir("no-args");
    let out = Command::new(bin())
        .env("LEGO_LEGACY_GAME_DATA", &empty)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .expect("run launcher");

    assert!(!out.status.success(), "no discs should exit failure");
    let stdout = String::from_utf8_lossy(&out.stdout);

    assert!(
        !stdout.contains('\x1b'),
        "piped/NO_COLOR output must not contain ANSI escapes:\n{stdout}"
    );
    assert!(
        stdout.contains(".===.") && stdout.contains("+------------------------------+"),
        "expected LEGO brick ASCII art banner:\n{stdout}"
    );
    for expected in [
        "LEGO Rock Raiders",
        "Hot Wheels Stunt Track Driver",
        "LEGO Alpha Team",
        "LEGO Island",
        "LEGO Chess",
        "LEGO Racers",
        "LEGOLAND",
    ] {
        assert!(
            stdout.contains(expected),
            "roster title `{expected}` missing from output:\n{stdout}"
        );
    }
    for forbidden in ["lego-ctrl", "lego-mcp"] {
        assert!(
            !stdout.contains(forbidden),
            "tool binary `{forbidden}` must never appear:\n{stdout}"
        );
    }
}

#[test]
fn leading_flags_without_title_enter_picker_instead_of_unknown_title() {
    let empty = empty_dir("leading-flags");
    let out = Command::new(bin())
        .args(["--audio", "--record"])
        .env("LEGO_LEGACY_GAME_DATA", &empty)
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .output()
        .expect("run launcher --audio --record");

    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("no supported title"),
        "--audio must not be rejected as an unknown title; stderr:\n{stderr}"
    );
    assert!(
        stdout.contains(".===.") && stdout.contains("LEGO Racers"),
        "should render picker banner and roster; stdout:\n{stdout}"
    );
    assert!(
        stdout.contains("--audio"),
        "picker should indicate forwarded flags; stdout:\n{stdout}"
    );
}

#[test]
fn unknown_and_ambiguous_titles_fail_cleanly() {
    let empty = empty_dir("bad-titles");

    for bad in ["nonexistent", "lego-ctrl", "lego-mcp"] {
        let out = Command::new(bin())
            .arg(bad)
            .env("LEGO_LEGACY_GAME_DATA", &empty)
            .env("NO_COLOR", "1")
            .output()
            .expect("run launcher with unknown title");
        assert!(!out.status.success(), "`{bad}` should fail");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(
            stderr.contains(bad),
            "stderr should name `{bad}`:\n{stderr}"
        );
    }

    let amb = Command::new(bin())
        .arg("lego")
        .env("LEGO_LEGACY_GAME_DATA", &empty)
        .env("NO_COLOR", "1")
        .output()
        .expect("run launcher with ambiguous title");
    assert!(!amb.status.success(), "`lego` should fail as ambiguous");
    let stderr = String::from_utf8_lossy(&amb.stderr);
    assert!(
        stderr.contains("ambiguous") && stderr.contains("lego-racers"),
        "ambiguous stderr should list matching keys:\n{stderr}"
    );
}

#[cfg(unix)]
#[test]
fn short_names_and_leading_or_trailing_flags_forward_to_matching_binary() {
    let (_dir, launcher) = install_stub_dir(
        "short-names",
        &[
            "lego-rock-raiders",
            "hotwheels-stunt-track-driver",
            "lego-alpha-team",
            "lego-island",
            "lego-chess",
            "lego-racers",
            "legoland",
        ],
    );

    let cases: &[(&[&str], &str, &str)] = &[
        (&["racers", "--audio"], "lego-racers", "--audio"),
        (&["legoland", "--audio"], "legoland", "--audio"),
        (
            &["--audio", "racers", "--record"],
            "lego-racers",
            "--audio --record",
        ),
        (&["chess", "--headless"], "lego-chess", "--headless"),
        (&["island"], "lego-island", ""),
        (&["alpha-team", "--audio"], "lego-alpha-team", "--audio"),
        (
            &["rock-raiders", "--route", "m1.route"],
            "lego-rock-raiders",
            "--route m1.route",
        ),
        (
            &["hotwheels", "--audio"],
            "hotwheels-stunt-track-driver",
            "--audio",
        ),
        (&["lego-racers", "--help"], "lego-racers", "--help"),
        (
            &["--audio", "--route", "foo.route", "racers"],
            "lego-racers",
            "--audio --route foo.route",
        ),
    ];

    for (args, expected_bin, expected_args) in cases {
        let out = Command::new(&launcher)
            .args(*args)
            .output()
            .unwrap_or_else(|e| panic!("run {args:?}: {e}"));
        assert!(
            out.status.success(),
            "args {args:?} failed: stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let expected_line = format!("BIN={expected_bin} ARGS={expected_args}");
        assert_eq!(stdout.trim(), expected_line, "mismatch for args {args:?}");
    }
}

#[cfg(unix)]
fn run_piped_picker(
    name: &str,
    stub_key: &str,
    iso_rel: &str,
    flags: &[&str],
    choice: &str,
) -> Option<String> {
    use std::io::Write;
    use std::path::Path;

    let repo_game_data = std::env::var_os("LEGO_LEGACY_GAME_DATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("..")
                .join("game-data")
        });
    let iso = repo_game_data.join(iso_rel);
    if !iso.is_file() {
        let required = std::env::var("LEGO_LEGACY_REQUIRE_DISC").is_ok_and(|v| v != "0");
        assert!(
            !required,
            "LEGO_LEGACY_REQUIRE_DISC is set but {} is absent",
            iso.display()
        );
        eprintln!("skipping: no {} (rip your own discs there)", iso.display());
        return None;
    }

    let (_dir, launcher) = install_stub_dir(name, &[stub_key]);
    let mut child = Command::new(&launcher)
        .args(flags)
        .env("LEGO_LEGACY_GAME_DATA", &repo_game_data)
        .env("NO_COLOR", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn launcher");
    {
        let stdin = child.stdin.as_mut().expect("stdin");
        stdin.write_all(choice.as_bytes()).expect("write choice");
    }
    let out = child.wait_with_output().expect("wait launcher");
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "stderr:\n{stderr}\nstdout:\n{stdout}");
    Some(stdout)
}

#[cfg(unix)]
#[test]
fn piped_picker_forwards_leading_flags_when_disc_is_ready() {
    let Some(stdout) = run_piped_picker(
        "piped-picker",
        "lego-rock-raiders",
        "rock-raiders/masterpiece-rock-raiders/masterpiece-rock-raiders.iso",
        &["--audio", "--record"],
        "1\n",
    ) else {
        return;
    };
    assert!(
        stdout.contains("BIN=lego-rock-raiders ARGS=--audio --record"),
        "expected piped picker to forward flags to chosen title:\n{stdout}"
    );
}

#[cfg(unix)]
#[test]
fn piped_picker_lists_legoland_seventh_and_ready_when_disc_is_present() {
    let Some(stdout) = run_piped_picker(
        "piped-picker-legoland",
        "legoland",
        "legoland/masterpiece-legoland/masterpiece-legoland.iso",
        &["--headless"],
        "7\n",
    ) else {
        return;
    };
    assert!(
        stdout.lines().any(|l| {
            l.contains("7. LEGOLAND") && l.contains("[ready]") && l.contains("masterpiece-legoland")
        }),
        "expected `7. LEGOLAND` ready from masterpiece-legoland:\n{stdout}"
    );
    assert!(
        stdout.contains("BIN=legoland ARGS=--headless"),
        "expected piped picker choice 7 to launch legoland:\n{stdout}"
    );
}
