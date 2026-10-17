//! What the guest writes: kept in the title's profile, read back from there,
//! and standing in front of the pressing's own files.

use lego_legacy_core::install::Tree;
use std::path::PathBuf;

#[test]
fn a_save_outlives_the_run_that_wrote_it_and_the_disc_underneath_it_does_not_change() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("profile");
    let _ = std::fs::remove_dir_all(&dir);
    let mut tree = Tree::new();
    tree.declare("C:\\LEGORR\\Data\\config.cfg", b"from the disc".to_vec());

    // Without a profile the guest's disc is read-only, and says so rather
    // than counting a write it dropped.
    assert!(tree
        .write("C:\\LEGORR\\Data\\Saves\\0.sav", b"a save")
        .is_err());
    tree.set_profile(dir.clone());

    tree.write("C:\\LEGORR\\Data\\Saves\\0.sav", b"a save")
        .expect("the profile takes it");
    assert_eq!(
        tree.read("c:/legorr/data/saves/0.sav").as_deref(),
        Some(&b"a save"[..]),
        "spelled whichever way the guest asks for it"
    );
    assert!(
        dir.join("C/LEGORR/DATA/SAVES/0.SAV").exists(),
        "under the profile, the drive among the directories, and nowhere above it"
    );

    // What the guest wrote stands in front of what the pressing holds.
    tree.write("C:\\LEGORR\\Data\\config.cfg", b"changed")
        .expect("the profile takes it");
    assert_eq!(
        tree.read("C:\\LEGORR\\Data\\config.cfg").as_deref(),
        Some(&b"changed"[..])
    );

    assert!(tree.remove("C:\\LEGORR\\Data\\Saves\\0.sav"));
    assert_eq!(tree.read("C:\\LEGORR\\Data\\Saves\\0.sav"), None);
    // Deleting reaches only what the guest itself wrote: the pressing's own
    // file is still there underneath.
    assert!(tree.remove("C:\\LEGORR\\Data\\config.cfg"));
    assert_eq!(
        tree.read("C:\\LEGORR\\Data\\config.cfg").as_deref(),
        Some(&b"from the disc"[..]),
        "the disc's own comes back"
    );
}

/// Whether a file is there is what reading it would say, without reading it:
/// what `GetFileAttributesA` asks.
#[test]
fn a_file_is_there_exactly_when_reading_it_would_find_it() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("readable");
    let _ = std::fs::remove_dir_all(&dir);
    let mut tree = Tree::new();
    tree.declare("C:\\LEGORR\\Data\\config.cfg", Vec::new());
    tree.set_profile(dir);
    tree.write("C:\\LEGORR\\Data\\Saves\\0.sav", b"a save")
        .expect("the profile takes it");
    for path in [
        "c:/legorr/data/config.cfg",
        "C:\\LEGORR\\Data\\Saves\\0.sav",
        // A directory the guest's save made is not a file.
        "C:\\LEGORR\\Data\\Saves",
        "C:\\LEGORR\\nothing",
        // No pressing in the drive.
        "D:\\EXE\\LEGORR.EXE",
    ] {
        assert_eq!(tree.readable(path), tree.read(path).is_some(), "{path}");
    }
    assert!(tree.readable("C:\\LEGORR\\Data\\config.cfg"));
}
