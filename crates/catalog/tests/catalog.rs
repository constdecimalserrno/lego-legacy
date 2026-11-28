//! Cross-title catalog invariants and real-disc format/roster checks (#407).
//!
//! Consumes `lego_legacy_catalog::TITLES` and each title's single-source
//! `Title` declaration. Covers:
//! - all 7 titles' unique keys, unique display names, valid hashes, guides,
//!   `/api/meta`, `/api/controls`, `/api/goals` JSON serialization, and MIDI
//!   bank declarations;
//! - real-disc `roster::{look_for, find_bank}` checks (loose, cabinet, IS3,
//!   SafeDisc wrappers, wrong digest, missing member);
//! - real-disc InstallShield 5/6 cabinet tests (Rock Raiders & Alpha Team);
//! - real-disc InstallShield 3 archive tests (LEGO Chess `MAIN.Z` & `_SETUP.LIB`);
//! - real-disc Roland `GM16.DLS` parsing and synth determinism.

use lego_legacy_catalog::{
    GM16, HOT_WHEELS_STUNT_TRACK_DRIVER, LEGOLAND, LEGO_ALPHA_TEAM, LEGO_CHESS, ROCK_RAIDERS,
    TITLES,
};
use lego_legacy_core::cabinet::Cabinet;
use lego_legacy_core::control::json::parse_json;
use lego_legacy_core::control::{format_controls_json, format_goals_json, format_meta_json};
use lego_legacy_core::dls::Bank;
use lego_legacy_core::is3::Archive;
use lego_legacy_core::roster::{self, Found};
use lego_legacy_core::synth::Synth;
use lego_legacy_core::title::EntryImage;
use lego_legacy_core::{disc, hash, testkit, Title};
use std::collections::BTreeSet;

const ROCK_RAIDERS_WRAPPED: Title = Title {
    image: EntryImage {
        pressing: "rock-raiders",
        ..ROCK_RAIDERS.image
    },
    ..ROCK_RAIDERS
};

#[test]
fn every_title_in_the_catalog_has_unique_identity_valid_image_and_populated_guide() {
    assert_eq!(TITLES.len(), 7);
    let expected_order = [
        "lego-rock-raiders",
        "hotwheels-stunt-track-driver",
        "lego-alpha-team",
        "lego-island",
        "lego-chess",
        "lego-racers",
        "legoland",
    ];
    let actual_order: Vec<&str> = TITLES.iter().map(|t| t.key).collect();
    assert_eq!(actual_order, expected_order);

    // Hot Wheels uses one unified display name across launcher and /api/meta.
    assert_eq!(
        HOT_WHEELS_STUNT_TRACK_DRIVER.name,
        "Hot Wheels Stunt Track Driver"
    );

    let mut keys = BTreeSet::new();
    let mut names = BTreeSet::new();
    for title in TITLES {
        assert!(!title.key.is_empty() && !title.name.is_empty());
        assert!(keys.insert(title.key), "duplicate key {}", title.key);
        assert!(names.insert(title.name), "duplicate name {}", title.name);
        assert_eq!(
            title.image.sha256.len(),
            64,
            "{} names its image by 64-char sha256",
            title.key
        );
        assert!(
            !title.image.why.is_empty(),
            "{} says why that pressing",
            title.key
        );
        assert!(
            !title.guide.is_empty(),
            "{} declares a populated TitleGuide",
            title.key
        );
        for place in title.bank {
            assert_eq!(
                place.sha256.len(),
                64,
                "{} bank on {} names its bytes",
                title.key,
                place.pressing
            );
            assert!(
                !place.why.is_empty(),
                "{} bank on {} says why",
                title.key,
                place.pressing
            );
        }

        // Single-source `Title.name` is emitted consistently across meta/controls/goals JSON.
        let meta = parse_json(&format_meta_json(title)).expect("valid meta JSON");
        let meta_obj = meta.as_object().unwrap();
        assert_eq!(meta_obj["title"].as_str(), Some(title.key));
        assert_eq!(
            meta_obj["guide"].as_object().unwrap()["display_name"].as_str(),
            Some(title.name)
        );
        let controls = parse_json(&format_controls_json(title)).expect("valid controls JSON");
        assert_eq!(
            controls.as_object().unwrap()["display_name"].as_str(),
            Some(title.name)
        );
        let goals = parse_json(&format_goals_json(title)).expect("valid goals JSON");
        assert_eq!(
            goals.as_object().unwrap()["display_name"].as_str(),
            Some(title.name)
        );
    }

    // General MIDI bank is declared for LEGO Chess and LEGOLAND, and no other title.
    assert_eq!(LEGO_CHESS.bank, &GM16);
    assert_eq!(LEGOLAND.bank, &GM16);
    for title in TITLES
        .iter()
        .filter(|t| t.key != LEGO_CHESS.key && t.key != LEGOLAND.key)
    {
        assert!(title.bank.is_empty(), "{} has no MIDI bank", title.key);
    }
}

#[test]
fn a_pressing_that_is_there_and_right_is_ready_to_start() {
    if !testkit::rips(&ROCK_RAIDERS.image) {
        return;
    }
    let found = roster::look_for(&ROCK_RAIDERS);
    assert_eq!(
        found,
        Found::Runnable {
            pressing: "masterpiece-rock-raiders".to_owned()
        },
        "the Masterpiece pressing holds the unwrapped game"
    );
    assert!(found.to_string().contains("ready"));
}

#[test]
fn a_wrapped_pressing_is_named_rather_than_offered() {
    if !testkit::rips(&ROCK_RAIDERS_WRAPPED.image) {
        return;
    }
    let found = roster::look_for(&ROCK_RAIDERS_WRAPPED);
    match &found {
        Found::Wrapped { scheme, pressing } => {
            assert_eq!(scheme, "SafeDisc");
            assert_eq!(pressing, "rock-raiders");
        }
        other => panic!("found {other} instead of naming the wrapper"),
    }
    assert!(found.to_string().contains("will not be run"));
}

#[test]
fn a_pressing_holding_different_bytes_is_refused_by_its_digest() {
    if !testkit::rips(&ROCK_RAIDERS.image) {
        return;
    }
    let patched = Title {
        image: EntryImage {
            sha256: "0000000000000000000000000000000000000000000000000000000000000000",
            ..ROCK_RAIDERS.image
        },
        ..ROCK_RAIDERS
    };
    match roster::look_for(&patched) {
        Found::WrongBytes { found, .. } => {
            assert!(
                found.starts_with("62b67f5b"),
                "it says what it found: {found}"
            );
        }
        other => panic!("found {other} instead of refusing the bytes"),
    }
}

/// Alpha Team's image is inside its pressing's cabinet rather than on the
/// disc, and the launcher has to read it from there to know the bytes (#53).
#[test]
fn an_image_inside_the_cabinet_is_found_and_hashed_like_any_other() {
    if !testkit::rips(&LEGO_ALPHA_TEAM.image) {
        return;
    }
    assert_eq!(
        roster::look_for(&LEGO_ALPHA_TEAM),
        Found::Runnable {
            pressing: "alpha-team".to_owned()
        }
    );
}

/// The same title pointed at an image the cabinet does not hold says what it
/// looked for rather than starting something else (#53).
#[test]
fn an_image_the_cabinet_does_not_hold_is_named() {
    if !testkit::rips(&LEGO_ALPHA_TEAM.image) {
        return;
    }
    let absent = Title {
        image: EntryImage {
            path: "NoSuchThing.exe",
            ..LEGO_ALPHA_TEAM.image
        },
        ..LEGO_ALPHA_TEAM
    };
    match roster::look_for(&absent) {
        Found::Unreadable { why } => assert!(why.contains("NoSuchThing.exe"), "{why}"),
        other => panic!("found {other} instead of naming what it looked for"),
    }
}

/// LEGO Chess pointed at an image its `MAIN.Z` does not hold says what it
/// looked for, as the cabinet's refusal does (#143).
#[test]
fn an_image_the_archive_does_not_hold_is_named() {
    if !testkit::rips(&LEGO_CHESS.image) {
        return;
    }
    let absent = Title {
        image: EntryImage {
            path: "NoSuchThing.exe",
            ..LEGO_CHESS.image
        },
        ..LEGO_CHESS
    };
    match roster::look_for(&absent) {
        Found::Unreadable { why } => assert!(why.contains("NoSuchThing.exe"), "{why}"),
        other => panic!("found {other} instead of naming what it looked for"),
    }
}

/// Roland's bank is found where it lies loose on the player's own disc and
/// checked by its SHA-256: a rip that is not there, and a place whose file
/// does not hash to the sha256 it declares, are passed over for the next
/// place, and said when none holds it (#156).
#[test]
fn the_bank_is_found_on_the_first_rip_that_holds_it() {
    let Some(rip) = testkit::any_rip(&GM16) else {
        return;
    };
    let absent = EntryImage {
        pressing: "no-such-pressing",
        ..GM16[0]
    };
    let wrong = EntryImage {
        sha256: "not the bank's",
        ..*rip
    };
    let places = [absent, wrong, *rip];
    let (place, bytes) = roster::find_bank(&places).expect("on the present rip");
    assert_eq!((place.pressing, bytes.len()), (rip.pressing, 3_440_660));
    let err = roster::find_bank(&places[..2]).unwrap_err();
    assert_eq!(
        err,
        format!(
            "the {} rip holds other bytes at {} (3229b09b9d7d9f3f…)",
            rip.pressing, rip.path
        )
    );
}

// --- Migrated real-disc InstallShield 5/6 cabinet tests (from core/tests/cabinet.rs) ---

fn open_iso(image: &EntryImage) -> Option<disc::Iso> {
    testkit::rips(image)
        .then(|| disc::open_pressing(image.disc_dir, image.pressing).expect("the pressing opens"))
}

#[test]
fn the_header_names_every_file_the_installer_would_have_written() {
    let Some(mut iso) = open_iso(&ROCK_RAIDERS.image) else {
        return;
    };
    let header = iso.read("DATA1.HDR").expect("the header reads");
    let cabinet = Cabinet::open(&header).expect("this is a cabinet");

    assert_eq!(
        cabinet.entries().len(),
        1934,
        "every file the installer knows about"
    );
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

    let exe = cabinet
        .find("LegoRR.exe")
        .expect("the header names the executable");
    assert!(!exe.in_cabinet());
    assert_eq!(exe.expanded, 736_768, "the same 737 KB the disc holds");

    let loose = cabinet.entries().iter().filter(|e| !e.in_cabinet()).count();
    assert!(loose > 1000, "{loose} entries are loose on the disc");
}

#[test]
fn a_small_entry_unpacks_to_exactly_what_the_header_says() {
    let Some(mut iso) = open_iso(&ROCK_RAIDERS.image) else {
        return;
    };
    let header = iso.read("DATA1.HDR").expect("the header reads");
    let cabinet = Cabinet::open(&header).expect("this is a cabinet");
    let data = iso.read("DATA1.CAB").expect("the cabinet reads");
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
    let Some(mut iso) = open_iso(&ROCK_RAIDERS.image) else {
        return;
    };
    let header = iso.read("DATA1.HDR").expect("the header reads");
    let cabinet = Cabinet::open(&header).expect("this is a cabinet");
    let data = iso.read("DATA1.CAB").expect("the cabinet reads");
    let span = |e: &lego_legacy_core::cabinet::Entry| {
        data[e.offset as usize..][..e.compressed as usize].to_vec()
    };

    let first = cabinet.find("LegoRR0.wad").expect("the header names it");
    let bytes = cabinet.unpack(first, &span(first)).expect("it unpacks");
    assert_eq!(bytes.len(), 78_968_800);
    assert_eq!(&bytes[..4], b"WWAD");
    assert_eq!(
        hash::sha256_hex(&bytes),
        "e02fa495f144d31142ef01a15221b1b7865d2a8264c43700523f2b52f91eaf68"
    );

    let second = cabinet.find("LegoRR1.wad").expect("the header names it");
    let bytes = cabinet.unpack(second, &span(second)).expect("it unpacks");
    assert_eq!(bytes.len() as u32, second.expanded);
    assert_eq!(&bytes[..4], b"WWAD");
    assert_eq!(
        hash::sha256_hex(&bytes),
        "5a967d3147533c61390120f923cea2bd275cc29bc7a8563de21c62bd9de6229d"
    );

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

#[test]
fn a_version_6_header_names_its_files_their_sizes_and_their_volumes() {
    let Some(mut iso) = open_iso(&LEGO_ALPHA_TEAM.image) else {
        return;
    };
    let header = iso.read("DATA1.HDR").expect("the header reads");
    let cabinet = Cabinet::open(&header).expect("this is a cabinet");
    assert_eq!(cabinet.entries().len(), 64);

    let exe = cabinet.find("LEGOATeam.exe").expect("the header names it");
    assert!(exe.in_cabinet());
    assert_eq!(exe.expanded, 61_440);
    assert_eq!(exe.compressed, 21_824);
    assert_eq!(exe.offset, 108_308_115);
    assert_eq!(exe.volume, 2, "the game is in DATA2.CAB");

    let kernel = cabinet.find("iKernel.exe").expect("the header names it");
    assert_eq!(kernel.volume, 1, "the installer is in DATA1.CAB");

    let game = cabinet.entries().iter().filter(|e| e.volume == 2).count();
    assert_eq!(game, 47);
    let engine = cabinet.entries().iter().filter(|e| e.volume == 1).count();
    assert_eq!(engine, 17);

    let ambient = cabinet.find("Ambient.pac").expect("the header names it");
    assert_eq!(ambient.expanded, 126_598_304, "the largest entry there is");
}

#[test]
fn the_entry_image_unpacks_from_the_second_volume() {
    let Some(mut iso) = open_iso(&LEGO_ALPHA_TEAM.image) else {
        return;
    };
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

#[test]
fn a_placeholder_the_installer_would_have_filled_is_an_empty_file() {
    let Some(mut iso) = open_iso(&LEGO_ALPHA_TEAM.image) else {
        return;
    };
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

#[test]
fn a_cabinet_of_an_unknown_version_is_refused_by_number() {
    let Some(mut iso) = open_iso(&LEGO_ALPHA_TEAM.image) else {
        return;
    };
    let mut header = iso.read("DATA1.HDR").expect("the header reads");
    header[0x05] = 0x70;
    let Err(why) = Cabinet::open(&header) else {
        panic!("a version 7 cabinet was read as one this reader knows");
    };
    assert_eq!(
        why,
        lego_legacy_core::cabinet::CabinetError::UnknownVersion(7)
    );
}

// --- Migrated real-disc InstallShield 3 archive tests (from core/tests/is3.rs) ---

fn chess_archive(name: &str) -> Option<(Vec<u8>, Archive)> {
    let mut iso = open_iso(&LEGO_CHESS.image)?;
    let bytes = iso.read(name).expect("the archive reads");
    let archive = Archive::open(&bytes).expect("this is an InstallShield 3 archive");
    Some((bytes, archive))
}

#[test]
fn main_z_holds_the_whole_game_with_the_executable_at_its_root() {
    let Some((bytes, archive)) = chess_archive("MAIN.Z") else {
        return;
    };
    assert_eq!(archive.members().len(), 881);

    let exe = archive.find("Lego Chess.exe").expect("the table names it");
    assert_eq!((exe.directory.as_str(), exe.expanded), ("", 593_971));
    let image = archive.read(&bytes, "Lego Chess.exe").expect("it expands");
    assert_eq!(hash::sha256_hex(&image), LEGO_CHESS.image.sha256);
    assert_eq!(
        LEGO_CHESS.image.sha256,
        "7f5205be8b90343cb2a6088d0814cefc6031fda353145aa56b639ceaf79d0164"
    );

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
    let Some((bytes, archive)) = chess_archive("MAIN.Z") else {
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
    let Some((bytes, archive)) = chess_archive("_SETUP.LIB") else {
        return;
    };
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

// --- Migrated real-disc Roland GM16.DLS synth tests (from core/tests/synth.rs) ---

fn gm16_bank() -> Option<Bank> {
    let rip = testkit::any_rip(&GM16)?;
    let mut iso = disc::open_pressing(rip.disc_dir, rip.pressing).expect("the pressing opens");
    let bytes = rip.read(&mut iso).expect("the bank reads");
    assert_eq!(
        hash::sha256_hex(&bytes),
        rip.sha256,
        "the player's own bank"
    );
    Some(Bank::parse(&bytes).expect("a DLS Level 1 bank"))
}

#[test]
fn the_players_bank_reads_whole() {
    let Some(bank) = gm16_bank() else { return };
    assert_eq!(bank.instruments.len(), 235);
    assert_eq!(bank.waves.len(), 495);
    let kits: Vec<u8> = bank
        .instruments
        .iter()
        .filter(|i| i.drums)
        .map(|i| i.program)
        .collect();
    assert_eq!(kits, [0, 8, 16, 24, 25, 32, 40, 48, 56], "the GS kits");
    for program in 0..128 {
        assert!(bank
            .instruments
            .iter()
            .any(|i| !i.drums && i.bank == 0 && i.program == program));
    }
    let standard = &bank.instruments.iter().find(|i| i.drums).unwrap().regions;
    assert_eq!(standard.first().map(|r| r.keys), Some((27, 27)));
    assert_eq!(standard.last().map(|r| r.keys), Some((87, 87)));
}

fn render_scale(bank: Bank) -> (Vec<i32>, Vec<u64>) {
    const QUARTER: u64 = 22_050 / 4;
    let mut synth = Synth::new(bank);
    let mut starts = Vec::new();
    for (n, key) in [60u8, 62, 64, 65, 67, 69, 71, 72].into_iter().enumerate() {
        let at = n as u64 * QUARTER;
        synth.message(at, u32::from_le_bytes([0x90, key, 100, 0]));
        synth.message(at + QUARTER, u32::from_le_bytes([0x80, key, 0, 0]));
        starts.push(at);
    }
    for n in 0..8 {
        let at = (8 + n / 2) * QUARTER + (n % 2) * QUARTER / 2;
        synth.message(at, u32::from_le_bytes([0x99, 82, 100, 0]));
        starts.push(at);
    }
    let frames = (0..14 * QUARTER).flat_map(|f| synth.frame(f)).collect();
    (frames, starts)
}

#[test]
fn a_general_midi_scale_and_the_gs_shaker_sound_and_sound_the_same_every_time() {
    let Some(bank) = gm16_bank() else { return };
    let (frames, starts) = render_scale(bank.clone());
    for at in starts {
        let window = &frames[at as usize * 2..(at as usize + 22_050 / 8) * 2];
        let peak = window.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
        assert!(peak > 1000, "the note at frame {at} peaks at {peak}");
    }
    let bytes: Vec<u8> = frames.iter().flat_map(|s| s.to_le_bytes()).collect();
    let digest = hash::sha256_hex(&bytes);
    assert_eq!(
        render_scale(bank).0,
        frames,
        "the same messages, the same samples"
    );
    assert_eq!(
        digest,
        "68a20ae67b427edc74c2729a00af7ccd13c6777565a87ca2b53a18aac156f59c"
    );
}
