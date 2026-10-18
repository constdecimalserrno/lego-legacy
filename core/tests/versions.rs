//! ADR-0007: every crate carries its own version, and a crate that changed
//! since its release moves it.
//!
//! For each member of the workspace, its version has a heading in its
//! CHANGELOG. When a `<crate>-v<version>` tag exists and any file under the
//! crate differs from the newest one, its version is greater than that tag's.
//! A tree that is not a git checkout has no tags, and only the changelog is
//! checked.

use std::path::{Path, PathBuf};
use std::process::Command;

/// A version as major, minor and patch, which compares the way SemVer orders
/// them.
type Version = (u64, u64, u64);

/// `major.minor.patch`, each part digits only. Anything else, a pre-release
/// suffix included, is not a version this workspace uses.
fn parse(text: &str) -> Option<Version> {
    let mut parts = text.split('.').map(|p| {
        if p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        p.parse().ok()
    });
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// The directories the workspace's `members` list names.
fn members(root: &Path) -> Vec<String> {
    let workspace = std::fs::read_to_string(root.join("Cargo.toml"))
        .unwrap_or_else(|e| panic!("no workspace manifest under {}: {e}", root.display()));
    workspace
        .split_once("members = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(list, _)| {
            list.split('"')
                .skip(1)
                .step_by(2)
                .map(String::from)
                .collect()
        })
        .unwrap_or_default()
}

/// A crate's name and version, from its manifest's `[package]` table.
fn package(manifest: &str) -> (Option<&str>, Option<&str>) {
    let mut in_package = false;
    let (mut name, mut version) = (None, None);
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let quoted = |key: &str| {
            line.strip_prefix(key)
                .and_then(|v| v.strip_prefix(" = \""))
                .and_then(|v| v.strip_suffix('"'))
        };
        name = name.or_else(|| quoted("name"));
        version = version.or_else(|| quoted("version"));
    }
    (name, version)
}

/// Run git in `root`. `None` when it could not be started at all.
fn git(root: &Path, args: &[&str]) -> Option<std::process::Output> {
    Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .ok()
}

/// What is wrong with the versions of the workspace at `root`, a line each.
fn complaints(root: &Path) -> Vec<String> {
    let checkout = git(root, &["rev-parse", "--is-inside-work-tree"])
        .is_some_and(|o| o.status.success() && o.stdout.starts_with(b"true"));
    let mut wrong = Vec::new();
    for member in members(root) {
        let dir = root.join(&member);
        let manifest = std::fs::read_to_string(dir.join("Cargo.toml"))
            .unwrap_or_else(|e| panic!("no manifest in {member}: {e}"));
        let (Some(name), Some(version)) = package(&manifest) else {
            wrong.push(format!(
                "{member}/Cargo.toml names no `name` and `version` of its own"
            ));
            continue;
        };
        let Some(current) = parse(version) else {
            wrong.push(format!(
                "{name}'s version `{version}` is not major.minor.patch"
            ));
            continue;
        };

        let changelog = std::fs::read_to_string(dir.join("CHANGELOG.md")).unwrap_or_default();
        let headed = changelog.lines().any(|l| {
            l.strip_prefix("## ")
                .and_then(|h| h.split_whitespace().next())
                == Some(version)
        });
        if !headed {
            wrong.push(format!(
                "{name} is at {version} and {member}/CHANGELOG.md has no `## {version}` heading"
            ));
        }

        if !checkout {
            continue;
        }
        let prefix = format!("{name}-v");
        let listed = git(root, &["tag", "--list", &format!("{prefix}*")])
            .filter(|o| o.status.success())
            .unwrap_or_else(|| panic!("git could not list {name}'s tags"));
        let mut newest: Option<(Version, String)> = None;
        for tag in String::from_utf8_lossy(&listed.stdout).lines() {
            match tag.strip_prefix(&prefix).and_then(parse) {
                Some(v) if newest.as_ref().is_none_or(|(n, _)| v > *n) => {
                    newest = Some((v, tag.to_owned()));
                }
                Some(_) => {}
                None => wrong.push(format!("the tag {tag} does not name a version")),
            }
        }
        let Some((tagged, tag)) = newest else {
            continue;
        };
        let diff = git(
            root,
            &[
                "diff",
                "--quiet",
                "--no-ext-diff",
                "--no-textconv",
                &format!("refs/tags/{tag}"),
                "--",
                &member,
            ],
        )
        .and_then(|o| o.status.code())
        .unwrap_or_else(|| panic!("git could not compare {member} with {tag}"));
        match diff {
            0 => {}
            1 if current > tagged => {}
            1 => wrong.push(format!(
                "{member} changed since {tag} and {name} is still at {version}: \
                 move it past the tag, and say why under its own heading in the CHANGELOG"
            )),
            _ => panic!("git could not compare {member} with {tag}"),
        }
    }
    wrong
}

#[test]
fn every_crate_has_a_changelog_heading_and_moved_its_version_if_it_changed_since_its_tag() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    assert!(
        members(&root).len() > 1,
        "the workspace's members read as {:?}",
        members(&root)
    );
    let wrong = complaints(&root);
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

/// A one-crate workspace in a git repository of its own, at version
/// `version` with a heading for `headed`. `None` when git cannot be started,
/// which skips the case the way a tree that is not a checkout skips the real
/// one.
fn fixture(case: &str, version: &str, headed: &str) -> Option<PathBuf> {
    let root = PathBuf::from(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("versions-{case}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("a")).expect("the fixture's directory");
    std::fs::write(root.join("Cargo.toml"), "[workspace]\nmembers = [\"a\"]\n")
        .expect("the workspace manifest");
    std::fs::write(root.join("a/src.rs"), "// one\n").expect("a source file");
    set(&root, version, headed);
    in_fixture(&root, &["init", "--quiet"])?;
    in_fixture(&root, &["add", "--all"])?;
    in_fixture(&root, &["commit", "--quiet", "--message", "release"])?;
    Some(root)
}

/// Write the fixture's crate at `version`, with a CHANGELOG heading for
/// `headed`.
fn set(root: &Path, version: &str, headed: &str) {
    std::fs::write(
        root.join("a/Cargo.toml"),
        format!("[package]\nname = \"a\"\nversion = \"{version}\"\n"),
    )
    .expect("the crate's manifest");
    std::fs::write(
        root.join("a/CHANGELOG.md"),
        format!("# Changelog\n\n## {headed} — 2026-10-18\n\nWhat changed.\n"),
    )
    .expect("the crate's changelog");
}

/// Run git in the fixture with none of the person's own configuration, so a
/// signing key or a hook of theirs never reaches it.
fn in_fixture(root: &Path, args: &[&str]) -> Option<()> {
    let status = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_AUTHOR_NAME", "lego-legacy")
        .env("GIT_AUTHOR_EMAIL", "lego-legacy@invalid")
        .env("GIT_COMMITTER_NAME", "lego-legacy")
        .env("GIT_COMMITTER_EMAIL", "lego-legacy@invalid")
        .status()
        .ok()?;
    assert!(status.success(), "git {args:?} failed in the fixture");
    Some(())
}

#[test]
fn a_crate_changed_since_its_newest_tag_must_move_past_it() {
    let Some(root) = fixture("changed", "1.10.0", "1.10.0") else {
        eprintln!("git cannot be started; skipped");
        return;
    };
    // Newest by version, not by name: 1.10.0 lists before 1.2.0.
    in_fixture(&root, &["tag", "a-v1.2.0"]).expect("git");
    in_fixture(&root, &["tag", "a-v1.10.0"]).expect("git");
    assert_eq!(complaints(&root), Vec::<String>::new(), "nothing changed");

    std::fs::write(root.join("a/src.rs"), "// two\n").expect("a change");
    let wrong = complaints(&root);
    assert_eq!(wrong.len(), 1, "{wrong:?}");
    assert!(
        wrong[0].contains("changed since a-v1.10.0") && wrong[0].contains("still at 1.10.0"),
        "{wrong:?}"
    );

    set(&root, "1.10.1", "1.10.1");
    assert_eq!(complaints(&root), Vec::<String>::new(), "a patch moves it");

    in_fixture(&root, &["tag", "a-v2.0.0-rc.1"]).expect("git");
    let wrong = complaints(&root);
    assert_eq!(
        wrong,
        ["the tag a-v2.0.0-rc.1 does not name a version"],
        "a tag in the crate's name is a version or a mistake"
    );
}

#[test]
fn a_version_with_no_changelog_heading_is_refused() {
    let Some(root) = fixture("unheaded", "1.1.0", "1.0.0") else {
        eprintln!("git cannot be started; skipped");
        return;
    };
    let wrong = complaints(&root);
    assert_eq!(wrong.len(), 1, "{wrong:?}");
    assert!(
        wrong[0].contains("is at 1.1.0") && wrong[0].contains("no `## 1.1.0` heading"),
        "{wrong:?}"
    );

    // A heading that only starts with the version is another version's.
    set(&root, "1.1.0", "1.1.01");
    assert_eq!(complaints(&root).len(), 1);

    set(&root, "1.1.0", "1.1.0");
    assert_eq!(complaints(&root), Vec::<String>::new());
}
