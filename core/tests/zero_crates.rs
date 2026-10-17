//! ADR-0001: zero external crates, forever.
//!
//! The gate is the lock file: it may name the workspace's own members and
//! nothing else. A crate that arrives as a dependency of a dependency shows up
//! here just the same as one added by hand.

use std::path::Path;

#[test]
fn the_lock_file_names_only_this_workspace() {
    let lock_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("Cargo.lock");
    let lock = std::fs::read_to_string(&lock_path)
        .unwrap_or_else(|e| panic!("no lock file at {}: {e}", lock_path.display()));

    // A package fetched from anywhere, a registry or a repository, carries a
    // `source` saying where; a workspace member has none. So a new title is a
    // new member and not an edit here.
    let foreign: Vec<&str> = lock
        .split("[[package]]")
        .filter(|p| p.lines().any(|l| l.starts_with("source = ")))
        .filter_map(|p| p.lines().find_map(|l| l.strip_prefix("name = ")))
        .collect();
    assert!(
        foreign.is_empty(),
        "Cargo.lock resolves crates that are not ours: {foreign:?}. \
         ADR-0001 says the emulator links nothing it did not write."
    );
}

#[test]
fn no_manifest_declares_a_dependency_that_is_not_a_path_to_a_member() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let workspace = std::fs::read_to_string(root.join("Cargo.toml"))
        .unwrap_or_else(|e| panic!("no workspace manifest: {e}"));
    // The workspace's own manifest, and every member's its `members` list
    // names, so a new title is checked the day it joins.
    let members: Vec<&str> = workspace
        .split_once("members = [")
        .and_then(|(_, rest)| rest.split_once(']'))
        .map(|(list, _)| list.split('"').skip(1).step_by(2).collect())
        .unwrap_or_default();
    assert!(
        members.contains(&"core"),
        "the workspace's members read as {members:?}"
    );
    let manifests = std::iter::once(root.join("Cargo.toml"))
        .chain(members.iter().map(|m| root.join(m).join("Cargo.toml")));
    for manifest in manifests {
        let text = std::fs::read_to_string(&manifest)
            .unwrap_or_else(|e| panic!("no manifest at {}: {e}", manifest.display()));
        let mut in_deps = false;
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                in_deps = line.contains("dependencies");
                continue;
            }
            if !in_deps || line.is_empty() || line.starts_with('#') {
                continue;
            }
            assert!(
                line.contains("path = "),
                "{} declares `{line}`, which is not a path to a workspace member",
                manifest.display()
            );
        }
    }
}
