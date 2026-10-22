//! LEGO Chess's InstallShield 3 archives, read out of its disc (#143).

use lego_legacy_core::is3::Archive;
use lego_legacy_core::{disc, hash, roster, testkit};

/// An archive of the Masterpiece pressing, whole, with its table read.
fn archive(name: &str) -> Option<(Vec<u8>, Archive)> {
    let image = &roster::LEGO_CHESS.image;
    if !testkit::rips(image) {
        return None;
    }
    let mut iso = disc::open_pressing(image.disc_dir, image.pressing).expect("the pressing opens");
    let bytes = iso.read(name).expect("the archive reads");
    let archive = Archive::open(&bytes).expect("this is an InstallShield 3 archive");
    Some((bytes, archive))
}

#[test]
fn main_z_holds_the_whole_game_with_the_executable_at_its_root() {
    let Some((bytes, archive)) = archive("MAIN.Z") else {
        return;
    };
    assert_eq!(archive.members().len(), 881);

    // The entry image, by its own bytes: at the root, as the setup's package
    // list has it too.
    let exe = archive.find("Lego Chess.exe").expect("the table names it");
    assert_eq!((exe.directory.as_str(), exe.expanded), ("", 593_971));
    let image = archive.read(&bytes, "Lego Chess.exe").expect("it expands");
    assert_eq!(hash::sha256_hex(&image), roster::LEGO_CHESS.image.sha256);
    assert_eq!(
        roster::LEGO_CHESS.image.sha256,
        "7f5205be8b90343cb2a6088d0814cefc6031fda353145aa56b639ceaf79d0164"
    );

    // Its renderers beside it, and its data in the directories it opens them
    // from. This pressing's table is in order, so these pass however a reader
    // assigns directories; the rule that a file's own record names its
    // directory is held by `is3`'s unit test, over an archive listed out of
    // order as the original pressing's is.
    for path in [
        "KLIB_DirectX_Plugin.dll",
        "KLIB_Software_Plugin.dll",
        "Advisors\\redking.avi",
        "staunton\\boardsquares\\chess1.bmp",
        "staunton\\red\\red0000.bmp",
        "profiles\\saves\\readme.txt",
    ] {
        assert!(archive.find(path).is_some(), "{path} is where it belongs");
    }
}

#[test]
fn every_member_of_main_z_expands_to_the_size_its_table_gives_it() {
    let Some((bytes, archive)) = archive("MAIN.Z") else {
        return;
    };
    let mut total = 0;
    for member in archive.members() {
        let expanded = archive
            .read(&bytes, &member.path())
            .unwrap_or_else(|e| panic!("{} would not expand: {e}", member.path()));
        assert_eq!(expanded.len(), member.expanded as usize);
        total += expanded.len();
    }
    assert_eq!(total, 39_471_251, "the whole game, expanded");
}

#[test]
fn text_txt_and_the_rest_of_the_setup_s_own_library_expand() {
    let Some((bytes, archive)) = archive("_SETUP.LIB") else {
        return;
    };
    // The setup's engine and its own resources, and the game's strings,
    // each to the size the table gives it.
    assert_eq!(archive.members().len(), 7);
    for member in archive.members() {
        let expanded = archive
            .read(&bytes, &member.path())
            .unwrap_or_else(|e| panic!("{} would not expand: {e}", member.path()));
        assert_eq!(
            expanded.len(),
            member.expanded as usize,
            "{}",
            member.path()
        );
    }
    let text = archive.read(&bytes, "Text.txt").expect("it expands");
    assert_eq!(text.len(), 5_645);
    assert_eq!(
        hash::sha256_hex(&text),
        "7c7a5118710a1c0f700c7bd955c76403dfcca5dd05e6ce2708b8f4b492d86d3c"
    );
}
