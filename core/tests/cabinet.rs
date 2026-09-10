//! The pressing's archive, read out of the disc.

use lego_legacy_core::cabinet::Cabinet;
use lego_legacy_core::{disc, hash};

fn pressing() -> Option<disc::Iso> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../game-data");
    std::env::set_var("LEGO_LEGACY_GAME_DATA", &root);
    match disc::open_pressing("rock-raiders", "masterpiece-rock-raiders") {
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
