//! The roster: which titles this emulator serves.
//!
//! A title's own crate declares the machine it runs on; what a title *is* — its
//! key, its binary, and the bytes of the image that makes it itself — is here,
//! because the launcher needs it too and it must not be written down twice.

use crate::title::{EntryImage, Kept};

/// One title, as everything outside its own crate needs to know it.
pub struct Entry {
    /// Roster key, which is also the name of the title's binary.
    pub key: &'static str,
    /// What a person calls it.
    pub name: &'static str,
    /// The image it runs.
    pub image: EntryImage,
    /// The pressing whose audio tracks it plays, when it has any.
    pub red_book: Option<&'static str>,
    /// Where the General MIDI bank its music is played with may be, the
    /// first found played; none for a title that plays no MIDI (ADR-0008).
    pub bank: &'static [EntryImage],
}

/// Roland's `GM16.DLS`, the bank the Microsoft GS Wavetable SW Synth plays
/// (ADR-0008), where it lies loose on the player's own discs: byte for byte
/// the same, with Roland's `GMREADME.TXT` beside it, on both pressings of
/// LEGO Racers, the disc #156 names, and on both of Rock Raiders, whose
/// Masterpiece every Rock Raiders player has already ripped. It is read
/// there, as an entry image is, and never copied out (golden rule 6).
pub const GM16: [EntryImage; 4] = [
    gm16(
        "racers",
        "masterpiece-racers",
        "REDIST/DIRECTX6/DIRECTX/GM16.DLS",
    ),
    gm16("racers", "racers", "REDIST/DIRECTX6/DIRECTX/GM16.DLS"),
    gm16(
        "rock-raiders",
        "masterpiece-rock-raiders",
        "DIRECTX6/DIRECTX6/DIRECTX/GM16.DLS",
    ),
    gm16(
        "rock-raiders",
        "rock-raiders",
        "DIRECTX6/DIRECTX6/DIRECTX/GM16.DLS",
    ),
];

const fn gm16(disc_dir: &'static str, pressing: &'static str, path: &'static str) -> EntryImage {
    EntryImage {
        disc_dir,
        pressing,
        path,
        kept: Kept::Loose,
        sha256: "3229b09b9d7d9f3f4793b0d9b34fe6abc75cfa4a2503c0c90f43ff651ba7f2c0",
        why: "Roland's General MIDI bank, which the Microsoft GS Wavetable SW Synth plays; \
              licensed by Roland for use with Microsoft operating system products only, and \
              the player's own copy",
    }
}

/// The first of a bank's places that holds it, and the bank's bytes; or why
/// none does. A rip that is not there, or whose file will not read or does
/// not hash to the bank's sha256, is passed over; when none holds the bank,
/// what each present rip held instead is said.
///
/// # Errors
/// When no place holds the bank: what each rip that is there held instead,
/// or that none of them is ripped.
pub fn find_bank(places: &[EntryImage]) -> Result<(&EntryImage, Vec<u8>), String> {
    let mut held = Vec::new();
    for place in places {
        let mut iso = match crate::disc::open_pressing(place.disc_dir, place.pressing) {
            Ok(iso) => iso,
            Err(crate::disc::DiscError::Missing(_)) => continue,
            Err(e) => {
                held.push(format!("the {} rip will not open: {e}", place.pressing));
                continue;
            }
        };
        match place.read(&mut iso) {
            Ok(bytes) => {
                let got = crate::hash::sha256_hex(&bytes);
                if got == place.sha256 {
                    return Ok((place, bytes));
                }
                held.push(format!(
                    "the {} rip holds other bytes at {} ({}…)",
                    place.pressing,
                    place.path,
                    &got[..16]
                ));
            }
            Err(e) => held.push(format!(
                "the {} rip will not give it up: {e}",
                place.pressing
            )),
        }
    }
    if held.is_empty() {
        return Err(match places {
            [] => "no bank pressings declared".to_owned(),
            [only] => format!("no rip of the {} pressing", only.pressing),
            many => {
                let pressings: Vec<&str> = many.iter().map(|p| p.pressing).collect();
                format!("no rip of the {} pressings", pressings.join(", "))
            }
        });
    }
    Err(held.join("; "))
}

/// LEGO Rock Raiders, the roster's first title.
pub const ROCK_RAIDERS: Entry = Entry {
    key: "lego-rock-raiders",
    name: "LEGO Rock Raiders",
    image: EntryImage {
        disc_dir: "rock-raiders",
        pressing: "masterpiece-rock-raiders",
        // Spelled as the cabinet's header names it, which is the name the
        // installer gives it and the one the title takes its own name, its
        // archives' and its window class's from (#93). The disc spells it in
        // capitals and is read without regard to case, as Windows reads one.
        path: "EXE/LegoRR.exe",
        kept: Kept::Loose,
        sha256: "62b67f5ba656c71412764d343c6a955d218e3b61f0e0f4758ad6086e971e64d4",
        why: "the original pressing's EXE/LEGORR.EXE is a 280 KB SafeDisc wrapper beside a \
              737 KB LEGORR.ICD holding the encrypted game; the Masterpiece pressing ships \
              that same game unwrapped, with an empty .ICD left behind (ADR-0006)",
    },
    // The original pressing carries three audio tracks and the Masterpiece
    // pressing carries none, so the music comes off the disc the executable
    // does not (ADR-0006).
    red_book: Some("rock-raiders"),
    bank: &[],
};

/// Hot Wheels Stunt Track Driver, the roster's second title.
pub const HOT_WHEELS: Entry = Entry {
    key: "hotwheels-stunt-track-driver",
    name: "Hot Wheels Stunt Track Driver",
    image: EntryImage {
        disc_dir: "hotwheels-stunt-track-driver",
        pressing: "hotwheels-stunt-track-driver",
        path: "SETUP/HOT98/STUNT.EXE",
        kept: Kept::Loose,
        sha256: "a938949d705e2e06af80c22bdd76b10b52df811342e5a5b28bc23ba716903850",
        why: "this title has one pressing and its executable is on the disc unwrapped, \
              beside the data an installer would have copied from the same directory",
    },
    // The disc carries one data track and no audio tracks: this title's music
    // is in its own files, not on the disc as Red Book audio.
    red_book: None,
    bank: &[],
};

/// LEGO Alpha Team, the roster's third title.
pub const ALPHA_TEAM: Entry = Entry {
    key: "lego-alpha-team",
    name: "LEGO Alpha Team",
    image: EntryImage {
        disc_dir: "alpha-team",
        pressing: "alpha-team",
        // A name in the pressing's cabinet, not a path on its disc: this title
        // ships nothing of itself loose (#53).
        path: "LEGOATeam.exe",
        kept: Kept::Cabinet,
        sha256: "36f77aa72b9345af2667f8e2c2395318bdbc9ac7649d378684a6bb0179eecc01",
        why: "both of this title's pressings carry the same cabinet byte for byte, so the \
              original is taken and the Masterpiece re-release buys nothing but an EA \
              registration tree, a PDF manual and menu fonts the game never reads (ADR-0006)",
    },
    // Neither pressing has an audio track: this title's music is in Audio.pac
    // and Ambient.pac, inside its own cabinet.
    red_book: None,
    bank: &[],
};

/// LEGO Island, the roster's fourth title (#112).
pub const LEGO_ISLAND: Entry = Entry {
    key: "lego-island",
    name: "LEGO Island",
    image: EntryImage {
        disc_dir: "lego-island",
        pressing: "lego-island",
        // The game itself, not `LEGOIsle.exe` beside it: that is the settings
        // launcher, byte for byte the disc's `CONFIG.EXE`, and all it does
        // once its dialog closes is start this one (#113).
        path: "DATA/disk/ISLE.EXE",
        kept: Kept::Loose,
        sha256: "5cf57c284973fce9d14f5677a2e4435fd989c5e938970764d00c8932ed5128ca",
        why: "this title has one pressing and its executable is on the disc unwrapped, beside \
              the engine and data its Wise installer copies from the same directory; the \
              settings launcher LEGOIsle.exe, which only starts this one, is not run",
    },
    // One data track and no audio tracks: this title's music is in its own
    // scripts and in MIDI it plays itself.
    red_book: None,
    bank: &[],
};

/// LEGO Chess, the roster's fifth title (#142).
pub const LEGO_CHESS: Entry = Entry {
    key: "lego-chess",
    name: "LEGO Chess",
    image: EntryImage {
        disc_dir: "chess",
        pressing: "masterpiece-chess",
        // At the root of the pressing's InstallShield 3 archive: nothing of
        // the game is loose on its disc, and its setup expands this archive
        // into the install directory whole (#143).
        path: "Lego Chess.exe",
        kept: Kept::Imploded("MAIN.Z"),
        sha256: "7f5205be8b90343cb2a6088d0814cefc6031fda353145aa56b639ceaf79d0164",
        why: "the maintainer chose the Masterpiece pressing (#142): its V2.06, linked in \
              February 1999, is a later build than the original pressing's V2.05, a different \
              executable, and neither is wrapped",
    },
    // One data track and no audio tracks: this title's music is General MIDI
    // in its own files.
    red_book: None,
    // Its four General MIDI tunes play through the MIDI mapper (#154, #156).
    bank: &GM16,
};

/// LEGO Racers, the roster's sixth title (#369).
pub const LEGO_RACERS: Entry = Entry {
    key: "lego-racers",
    name: "LEGO Racers",
    image: EntryImage {
        disc_dir: "racers",
        pressing: "masterpiece-racers",
        // Packed inside the pressing's InstallShield 5 cabinet (`DATA1.HDR` +
        // `DATA1.CAB`), beside `GolDP.dll` and `LEGO.JAM`: nothing of the game
        // itself lies loose on the disc (#369).
        path: "LEGORacers.exe",
        kept: Kept::Cabinet,
        sha256: "dcaea5ff899a34e7fbf95f3daefed16f55a1c9cd091b246e9fe71d83a2bf6597",
        why: "the original 1999 pressing packs a 239 KB SafeDisc v1 wrapper (LEGORacers.exe) \
              beside an encrypted 802 KB LEGORacers.icd in DATA1.CAB with 9,248 bad sectors on \
              disc; the Masterpiece pressing ships the 2001 unwrapped executable cleanly in \
              DATA1.CAB with zero bad sectors and byte-identical music and movies (ADR-0006)",
    },
    // Neither pressing has Red Book audio tracks: music is 50 High Voltage
    // Software ADPCM `.tun` files inside DATA1.CAB, and SFX are `.PCM` inside
    // `LEGO.JAM`.
    red_book: None,
    bank: &[],
};

/// LEGOLAND, the roster's seventh title (#393).
pub const LEGOLAND: Entry = Entry {
    key: "legoland",
    name: "LEGOLAND",
    image: EntryImage {
        disc_dir: "legoland",
        pressing: "masterpiece-legoland",
        // At the root of the pressing's InstallShield 3 archive (`main.z`, 331
        // members): `Legoland.exe`, `Lego.TTF`, `strings\stab.str`, `IMusic\*`,
        // `zbuffers\*`, `RollerCoaster\*`, and `AD_*.avi` are unpacked from
        // `main.z`, while the three `.res` volumes, `Speech\*.wav`, and
        // fullscreen `.avi` movies stay on disc (#393).
        path: "Legoland.exe",
        kept: Kept::Imploded("main.z"),
        sha256: "c50865b60bfcb26c0a7329a75fb772b10ae234af324f669901906e5abb0e2bd9",
        why: "the Masterpiece pressing ships the unwrapped Legoland.exe (MSVC 6.0, 2000-04-07), \
              with main.z, .res volumes, speech WAVs and movies byte-identical to the original \
              2000 pressing, in a clean single-track ISO with zero bad sectors and a Joliet tree \
              (ADR-0006)",
    },
    // Neither pressing carries Red Book audio tracks: speech is 1,266 MS ADPCM
    // WAVs in `Speech\` on disc and music is DirectMusic segments in `IMusic\`
    // inside `main.z`, played through Windows' General MIDI DLS bank (#399).
    red_book: None,
    bank: &GM16,
};

/// The roster's titles that have a crate, in the order they were taken: the
/// three v1 titles, LEGO Island, LEGO Chess, LEGO Racers and LEGOLAND. A title
/// joins when its own crate does, so the launcher never offers something that
/// cannot be started.
pub const ROSTER: &[&Entry] = &[
    &ROCK_RAIDERS,
    &HOT_WHEELS,
    &ALPHA_TEAM,
    &LEGO_ISLAND,
    &LEGO_CHESS,
    &LEGO_RACERS,
    &LEGOLAND,
];

/// What the launcher found when it went looking for a title.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Found {
    /// The declared image is there and is the right bytes.
    Runnable { pressing: String },
    /// The image is there and is wrapped in copy protection (ADR-0006).
    Wrapped { pressing: String, scheme: String },
    /// The image is there and is not the one this title declares.
    WrongBytes { pressing: String, found: String },
    /// No rip where one was expected, and where that was.
    Absent { looked_at: String },
    /// The rip is there and its executable cannot be read out of it: the
    /// image is damaged, is not an image, or does not hold it. Says which, as
    /// the machine would on starting it.
    Unreadable { why: String },
}

impl std::fmt::Display for Found {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Runnable { pressing } => write!(f, "ready, from {pressing}"),
            Self::Wrapped { pressing, scheme } => {
                write!(f, "{pressing} is wrapped in {scheme} and will not be run")
            }
            Self::WrongBytes { pressing, found } => {
                write!(
                    f,
                    "{pressing} holds different bytes ({}…)",
                    &found[..16.min(found.len())]
                )
            }
            Self::Absent { looked_at } => write!(f, "no rip at {looked_at}"),
            Self::Unreadable { why } => write!(f, "{why}"),
        }
    }
}

/// Go looking for a title's image.
#[must_use]
pub fn look_for(entry: &Entry) -> Found {
    let path = crate::disc::pressing_path(entry.image.disc_dir, entry.image.pressing);
    let mut iso = match crate::disc::Iso::open(&path) {
        Ok(iso) => iso,
        Err(crate::disc::DiscError::Missing(_)) => {
            return Found::Absent {
                looked_at: path.display().to_string(),
            };
        }
        Err(e) => return Found::Unreadable { why: e.to_string() },
    };
    match entry.image.read(&mut iso) {
        Ok(bytes) => judge(&entry.image, &bytes),
        Err(e) => Found::Unreadable { why: e.to_string() },
    }
}

/// What an image's bytes are, asked the same way by the launcher and by the
/// machine about to run them. The wrapper is looked for before the bytes are
/// hashed: a wrapped image is never the declared bytes, and its hash alone
/// would call it a bad rip.
#[must_use]
pub fn judge(image: &EntryImage, bytes: &[u8]) -> Found {
    let pressing = image.pressing.to_owned();
    if let Ok(inspected) = crate::pe::inspect(bytes) {
        if let Some(scheme) = crate::pe::wrapper_scheme(&inspected.sections) {
            return Found::Wrapped {
                pressing,
                scheme: scheme.to_owned(),
            };
        }
    }
    let digest = crate::hash::sha256_hex(bytes);
    if digest == image.sha256 {
        Found::Runnable { pressing }
    } else {
        Found::WrongBytes {
            pressing,
            found: digest,
        }
    }
}
