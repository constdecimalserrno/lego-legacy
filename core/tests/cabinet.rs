//! The pressing's archive, read out of the disc.

use lego_legacy_core::cabinet::Cabinet;
use lego_legacy_core::{disc, hash};

fn pressing() -> Option<disc::Iso> {
    open("rock-raiders", "masterpiece-rock-raiders")
}

/// Alpha Team's pressing, whose cabinet is version 6 (#52).
fn alpha() -> Option<disc::Iso> {
    open("alpha-team", "alpha-team")
}

fn open(title: &str, pressing: &str) -> Option<disc::Iso> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../game-data");
    std::env::set_var("LEGO_LEGACY_GAME_DATA", &root);
    match disc::open_pressing(title, pressing) {
        Ok(iso) => Some(iso),
        Err(_) => {
            let required = std::env::var("LEGO_LEGACY_REQUIRE_DISC").is_ok_and(|v| v != "0");
            assert!(
                !required,
                "LEGO_LEGACY_REQUIRE_DISC is set but the rips are absent"
            );
            eprintln!("skipping: no rips under {}", root.display());
            None
        }
    }
}

#[test]
fn the_header_names_every_file_the_installer_would_have_written() {
    let Some(mut iso) = pressing() else { return };
    let header = iso.read("DATA1.HDR").expect("the header reads");
    let cabinet = Cabinet::open(&header).expect("this is a cabinet");

    assert_eq!(
        cabinet.entries().len(),
        1934,
        "every file the installer knows about"
    );

    // The archives this title actually ships. It opens LegoRR0 through
    // LegoRR9 on startup and uses whichever are there, so two is not a
    // shortfall: it is what this pressing has.
    for n in 0..2 {
        let name = format!("LegoRR{n}.wad");
        let entry = cabinet
            .find(&name)
            .unwrap_or_else(|| panic!("{name} is named by the header"));
        assert!(entry.in_cabinet(), "{name} is packed rather than loose");
        assert!(entry.expanded > entry.compressed, "{name} is compressed");
    }
    for n in 2..10 {
        assert!(
            cabinet.find(&format!("LegoRR{n}.wad")).is_none(),
            "this pressing ships two archives, and the guest probes for ten"
        );
    }

    // The executable is named by the header too, and is not packed: it sits
    // loose on the disc, which is where the title's own image comes from.
    let exe = cabinet
        .find("LegoRR.exe")
        .expect("the header names the executable");
    assert!(!exe.in_cabinet());
    assert_eq!(exe.expanded, 736_768, "the same 737 KB the disc holds");

    // Most of what the header names is not packed at all: it sits loose on the
    // disc, and the tree reads it from there.
    let loose = cabinet.entries().iter().filter(|e| !e.in_cabinet()).count();
    assert!(loose > 1000, "{loose} entries are loose on the disc");
}

#[test]
fn something_that_is_not_a_cabinet_is_refused() {
    assert!(Cabinet::open(b"not a cabinet at all").is_err());
}

#[test]
fn a_small_entry_unpacks_to_exactly_what_the_header_says() {
    let Some(mut iso) = pressing() else { return };
    let header = iso.read("DATA1.HDR").expect("the header reads");
    let cabinet = Cabinet::open(&header).expect("this is a cabinet");
    let data = iso.read("DATA1.CAB").expect("the cabinet reads");
    // Each entry is unpacked from its own span.
    let span = |e: &lego_legacy_core::cabinet::Entry| {
        data[e.offset as usize..][..e.compressed as usize].to_vec()
    };

    let entry = cabinet.find("Ereg.Bin").expect("the header names it");
    let bytes = cabinet.unpack(entry, &span(entry)).expect("it unpacks");
    assert_eq!(
        bytes.len() as u32,
        entry.expanded,
        "as long as the header says"
    );
    assert_eq!(
        hash::sha256_hex(&bytes),
        "afa6f4941d357a0ae17b6f4367bfba0594b8360a13c85b97dd3e877f52300c3a"
    );
}

#[test]
fn the_title_s_own_archives_unpack_and_are_what_they_claim_to_be() {
    let Some(mut iso) = pressing() else { return };
    let header = iso.read("DATA1.HDR").expect("the header reads");
    let cabinet = Cabinet::open(&header).expect("this is a cabinet");
    let data = iso.read("DATA1.CAB").expect("the cabinet reads");
    // Each entry is unpacked from its own span.
    let span = |e: &lego_legacy_core::cabinet::Entry| {
        data[e.offset as usize..][..e.compressed as usize].to_vec()
    };

    // The first archive is 79 MB unpacked, which is most of this test's time,
    // so the rest are checked by their first chunk and their declared size.
    let first = cabinet.find("LegoRR0.wad").expect("the header names it");
    let bytes = cabinet.unpack(first, &span(first)).expect("it unpacks");
    assert_eq!(bytes.len(), 78_968_800);
    assert_eq!(
        &bytes[..4],
        b"WWAD",
        "a Rock Raiders archive says so in its first bytes"
    );
    assert_eq!(
        hash::sha256_hex(&bytes),
        "e02fa495f144d31142ef01a15221b1b7865d2a8264c43700523f2b52f91eaf68"
    );

    let second = cabinet.find("LegoRR1.wad").expect("the header names it");
    let bytes = cabinet.unpack(second, &span(second)).expect("it unpacks");
    assert_eq!(
        bytes.len() as u32,
        second.expanded,
        "as long as the header says"
    );
    assert_eq!(&bytes[..4], b"WWAD", "and an archive too");
    assert_eq!(
        hash::sha256_hex(&bytes),
        "5a967d3147533c61390120f923cea2bd275cc29bc7a8563de21c62bd9de6229d"
    );

    // Everything else the cabinet packs unpacks to the length its own header
    // declares. Nothing is taken on trust.
    let mut checked = 0;
    for entry in cabinet.entries().iter().filter(|e| e.in_cabinet()) {
        if entry.expanded > 4 * 1024 * 1024 {
            continue;
        }
        let bytes = cabinet
            .unpack(entry, &span(entry))
            .unwrap_or_else(|e| panic!("{} would not unpack: {e}", entry.name));
        assert_eq!(
            bytes.len() as u32,
            entry.expanded,
            "{} unpacked to a different length than declared",
            entry.name
        );
        checked += 1;
    }
    assert!(checked > 400, "only {checked} entries were checked");
}

/// Alpha Team's header is version 6, and reading it as version 5 found none of
/// its files rather than failing (#52).
#[test]
fn a_version_6_header_names_its_files_their_sizes_and_their_volumes() {
    let Some(mut iso) = alpha() else { return };
    let header = iso.read("DATA1.HDR").expect("the header reads");
    let cabinet = Cabinet::open(&header).expect("this is a cabinet");
    assert_eq!(cabinet.major_version(), 6);
    assert_eq!(cabinet.entries().len(), 64);

    // The title's own entry image, which is not on the disc as a file at all.
    let exe = cabinet.find("LEGOATeam.exe").expect("the header names it");
    assert!(exe.in_cabinet());
    assert_eq!(exe.expanded, 61_440);
    assert_eq!(exe.compressed, 21_824);
    assert_eq!(exe.offset, 108_308_115);
    assert_eq!(exe.volume, 2, "the game is in DATA2.CAB");

    // The installer's own engine is in the other volume.
    let kernel = cabinet.find("iKernel.exe").expect("the header names it");
    assert_eq!(kernel.volume, 1, "the installer is in DATA1.CAB");

    // Everything the title itself runs is in volume 2, and there are 47 of
    // them against the installer engine's 17.
    let game = cabinet.entries().iter().filter(|e| e.volume == 2).count();
    assert_eq!(game, 47);
    let engine = cabinet.entries().iter().filter(|e| e.volume == 1).count();
    assert_eq!(engine, 17);

    let ambient = cabinet.find("Ambient.pac").expect("the header names it");
    assert_eq!(ambient.expanded, 126_598_304, "the largest entry there is");
}

/// The entry image unpacks from the cabinet to exactly the bytes the roster
/// names it by (#52).
#[test]
fn the_entry_image_unpacks_from_the_second_volume() {
    let Some(mut iso) = alpha() else { return };
    let header = iso.read("DATA1.HDR").expect("the header reads");
    let cabinet = Cabinet::open(&header).expect("this is a cabinet");
    let exe = cabinet.find("LEGOATeam.exe").expect("the header names it");
    let span = iso
        .read_range("DATA2.CAB", exe.offset, exe.compressed as usize)
        .expect("the volume reads");
    let bytes = cabinet.unpack(exe, &span).expect("it unpacks");
    assert_eq!(bytes.len(), 61_440);
    assert_eq!(
        hash::sha256_hex(&bytes),
        "36f77aa72b9345af2667f8e2c2395318bdbc9ac7649d378684a6bb0179eecc01"
    );
}

/// A version 6 header names files it holds no bytes for: the installer would
/// have written them. They are empty, not missing and not corrupt (#52).
#[test]
fn a_placeholder_the_installer_would_have_filled_is_an_empty_file() {
    let Some(mut iso) = alpha() else { return };
    let header = iso.read("DATA1.HDR").expect("the header reads");
    let cabinet = Cabinet::open(&header).expect("this is a cabinet");

    let config = cabinet.find("Config.txt").expect("the header names it");
    assert_eq!(config.expanded, 0);
    assert_eq!(
        cabinet.unpack(config, &[]).expect("it unpacks"),
        Vec::<u8>::new()
    );

    let empty = cabinet
        .entries()
        .iter()
        .filter(|e| e.in_cabinet() && e.compressed == 0)
        .count();
    assert_eq!(empty, 7);
}

/// A version this reader has not been taught is refused by number rather than
/// read as one it has (#52).
#[test]
fn a_cabinet_of_an_unknown_version_is_refused_by_number() {
    let Some(mut iso) = alpha() else { return };
    let mut header = iso.read("DATA1.HDR").expect("the header reads");
    header[0x05] = 0x70; // 0x0100600c becomes a version 7
    let Err(why) = Cabinet::open(&header) else {
        panic!("a version 7 cabinet was read as one this reader knows");
    };
    assert_eq!(
        why,
        lego_legacy_core::cabinet::CabinetError::UnknownVersion(7)
    );
}
