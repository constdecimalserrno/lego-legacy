//! Generic `core::roster` lookup, image judging, and MIDI bank discovery tests.
//!
//! Strictly title-agnostic (Golden Rule 4): every test uses synthetic [`Title`]
//! and [`EntryImage`] fixtures with zero concrete game titles or disc rips.
//! Cross-title real-disc roster/cabinet invariants live in `crates/catalog/tests/`.

use lego_legacy_core::hash;
use lego_legacy_core::roster::{self, Found};
use lego_legacy_core::title::{EntryImage, Kept};
use lego_legacy_core::Title;

const SYNTHETIC_IMAGE: EntryImage = EntryImage {
    disc_dir: "synthetic-disc",
    pressing: "synthetic-pressing",
    path: "GAME.EXE",
    kept: Kept::Loose,
    sha256: "0000000000000000000000000000000000000000000000000000000000000000",
    why: "synthetic test fixture",
};

const SYNTHETIC_TITLE: Title = Title {
    key: "synthetic-title",
    name: "Synthetic Title",
    image: SYNTHETIC_IMAGE,
    ..Title::STUB
};

/// Build a minimal valid 32-bit x86 PE image with the given section names.
fn synthetic_pe(section_names: &[&[u8; 8]]) -> Vec<u8> {
    let pe_off: usize = 0x80;
    let opt_size: usize = 0x60;
    let table_off = pe_off + 24 + opt_size;
    let total = table_off + section_names.len() * 40;
    let mut bytes = vec![0u8; total];
    bytes[0..2].copy_from_slice(b"MZ");
    bytes[0x3c..0x40].copy_from_slice(&(pe_off as u32).to_le_bytes());
    bytes[pe_off..pe_off + 4].copy_from_slice(b"PE\0\0");
    // Machine = IMAGE_FILE_MACHINE_I386 (0x014c)
    bytes[pe_off + 4..pe_off + 6].copy_from_slice(&0x014cu16.to_le_bytes());
    // NumberOfSections
    bytes[pe_off + 6..pe_off + 8].copy_from_slice(&(section_names.len() as u16).to_le_bytes());
    // SizeOfOptionalHeader
    bytes[pe_off + 20..pe_off + 22].copy_from_slice(&(opt_size as u16).to_le_bytes());
    // Optional header magic = PE32 (0x010b)
    bytes[pe_off + 24..pe_off + 26].copy_from_slice(&0x010bu16.to_le_bytes());
    for (i, name) in section_names.iter().enumerate() {
        let at = table_off + i * 40;
        bytes[at..at + 8].copy_from_slice(*name);
    }
    bytes
}

#[test]
fn judge_classifies_runnable_wrong_bytes_and_wrapped_images() {
    let payload = b"synthetic unwrapped executable bytes";
    let digest = Box::leak(hash::sha256_hex(payload).into_boxed_str());
    let matching = EntryImage {
        sha256: digest,
        ..SYNTHETIC_IMAGE
    };

    let ok = roster::judge(&matching, payload);
    assert_eq!(
        ok,
        Found::Runnable {
            pressing: "synthetic-pressing".to_owned()
        }
    );
    assert!(ok.to_string().contains("ready"));

    let wrong = roster::judge(&matching, b"different bytes");
    match &wrong {
        Found::WrongBytes { pressing, found } => {
            assert_eq!(pressing, "synthetic-pressing");
            assert_eq!(found, &hash::sha256_hex(b"different bytes"));
        }
        other => panic!("expected WrongBytes, got {other}"),
    }
    assert!(wrong.to_string().contains("holds different bytes"));

    // A SafeDisc-wrapped PE (`.txt` + `.txt2`) is named as Wrapped BEFORE hashing,
    // even if its SHA-256 matches nothing (or matches the declared hash).
    let wrapped_pe = synthetic_pe(&[b".txt\0\0\0\0", b".txt2\0\0\0"]);
    let wrapped_digest = Box::leak(hash::sha256_hex(&wrapped_pe).into_boxed_str());
    let wrapped_img = EntryImage {
        sha256: wrapped_digest,
        ..SYNTHETIC_IMAGE
    };
    let wrapped = roster::judge(&wrapped_img, &wrapped_pe);
    assert_eq!(
        wrapped,
        Found::Wrapped {
            pressing: "synthetic-pressing".to_owned(),
            scheme: "SafeDisc".to_owned(),
        }
    );
    assert!(wrapped.to_string().contains("will not be run"));
}

#[test]
fn a_pressing_that_is_not_there_says_where_it_looked() {
    let absent = Title {
        image: EntryImage {
            pressing: "no-such-pressing",
            ..SYNTHETIC_IMAGE
        },
        ..SYNTHETIC_TITLE
    };
    match roster::look_for(&absent) {
        Found::Absent { looked_at } => {
            assert!(
                looked_at.contains("no-such-pressing"),
                "looked at {looked_at}"
            );
        }
        other => panic!("found {other} instead of saying where it looked"),
    }
}

/// A damaged rip whose volume descriptor is sound, with a root directory of
/// `len` bytes at sector 17 that starts with `record`.
fn damaged_rip(name: &str, len: u32, record: &[u8]) -> std::path::PathBuf {
    let mut image = vec![0u8; 18 * 2048];
    let pvd = 16 * 2048;
    image[pvd] = 1;
    image[pvd + 1..pvd + 6].copy_from_slice(b"CD001");
    image[pvd + 156 + 2..pvd + 156 + 6].copy_from_slice(&17u32.to_le_bytes());
    image[pvd + 156 + 10..pvd + 156 + 14].copy_from_slice(&len.to_le_bytes());
    image[17 * 2048..17 * 2048 + record.len()].copy_from_slice(record);
    let path = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::write(&path, image).expect("the rip is written");
    path
}

#[test]
fn a_damaged_rip_is_refused_by_name_rather_than_read_past() {
    // A record twenty bytes long, shorter than the thirty-three every record
    // starts with; and a root directory two gigabytes long, in an image of
    // thirty-six kilobytes.
    let truncated = damaged_rip("truncated-record.iso", 2048, &[20]);
    let enormous = damaged_rip("enormous-directory.iso", 0x8000_0000, &[]);
    for path in [truncated, enormous] {
        let mut iso = lego_legacy_core::disc::Iso::open(&path).expect("the descriptor is sound");
        let err = iso
            .list("")
            .expect_err("the directory is damaged")
            .to_string();
        assert!(
            err.contains(&path.display().to_string()) && err.contains("ripped again"),
            "names the image and says what to do: {err}"
        );
    }
}

/// The launcher lists a damaged rip as damaged, not as missing.
#[test]
fn a_damaged_rip_is_listed_as_damaged() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("listed-damaged");
    std::fs::create_dir_all(dir.join("x")).expect("the directory is made");
    damaged_rip("listed-damaged/x/x.iso", 2048, &[20]);
    let damaged = Title {
        image: EntryImage {
            // An absolute directory, which the game-data root joins as itself.
            disc_dir: Box::leak(dir.display().to_string().into_boxed_str()),
            pressing: "x",
            ..SYNTHETIC_IMAGE
        },
        ..SYNTHETIC_TITLE
    };
    match roster::look_for(&damaged) {
        Found::Unreadable { why } => assert!(why.contains("ripped again"), "{why}"),
        other => panic!("found {other} instead of saying the rip is damaged"),
    }
}

/// Without any game disc, `find_bank` reports empty declarations, missing
/// pressings (singular and plural), and damaged or non-ISO rips joined in
/// order.
#[test]
fn find_bank_reports_missing_and_damaged_rips_without_a_disc() {
    assert_eq!(
        roster::find_bank(&[]).unwrap_err(),
        "no bank pressings declared"
    );
    let missing = [
        EntryImage {
            pressing: "no-such-pressing-a",
            ..SYNTHETIC_IMAGE
        },
        EntryImage {
            pressing: "no-such-pressing-b",
            ..SYNTHETIC_IMAGE
        },
    ];
    assert_eq!(
        roster::find_bank(&missing[..1]).unwrap_err(),
        "no rip of the no-such-pressing-a pressing"
    );
    assert_eq!(
        roster::find_bank(&missing).unwrap_err(),
        "no rip of the no-such-pressing-a, no-such-pressing-b pressings"
    );
    // One non-ISO file (fails to open) and one damaged ISO (fails to read).
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("bank-damaged");
    std::fs::create_dir_all(dir.join("bad-iso")).expect("dir");
    std::fs::create_dir_all(dir.join("bad-dir")).expect("dir");
    std::fs::write(dir.join("bad-iso/bad-iso.iso"), b"not an iso").expect("write");
    damaged_rip("bank-damaged/bad-dir/bad-dir.iso", 2048, &[20]);
    let disc_dir: &'static str = Box::leak(dir.display().to_string().into_boxed_str());
    let absent = EntryImage {
        pressing: "no-such-pressing-a",
        ..SYNTHETIC_IMAGE
    };
    let bad_open = EntryImage {
        disc_dir,
        pressing: "bad-iso",
        ..SYNTHETIC_IMAGE
    };
    let bad_read = EntryImage {
        disc_dir,
        pressing: "bad-dir",
        ..SYNTHETIC_IMAGE
    };
    let err = roster::find_bank(&[absent, bad_open, bad_read]).unwrap_err();
    assert!(
        err.contains("the bad-iso rip will not open:")
            && err.contains("; the bad-dir rip will not give it up:"),
        "both present-rip failures reported in order: {err}"
    );
}
