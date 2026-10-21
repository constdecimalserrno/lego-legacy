//! The roster: which titles this emulator serves.
//!
//! A title's own crate declares the machine it runs on; what a title *is* — its
//! key, its binary, and the bytes of the image that makes it itself — is here,
//! because the launcher needs it too and it must not be written down twice.

use crate::title::EntryImage;

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
        in_cabinet: false,
        sha256: "62b67f5ba656c71412764d343c6a955d218e3b61f0e0f4758ad6086e971e64d4",
        why: "the original pressing's EXE/LEGORR.EXE is a 280 KB SafeDisc wrapper beside a \
              737 KB LEGORR.ICD holding the encrypted game; the Masterpiece pressing ships \
              that same game unwrapped, with an empty .ICD left behind (ADR-0006)",
    },
    // The original pressing carries three audio tracks and the Masterpiece
    // pressing carries none, so the music comes off the disc the executable
    // does not (ADR-0006).
    red_book: Some("rock-raiders"),
};

/// Hot Wheels Stunt Track Driver, the roster's second title.
pub const HOT_WHEELS: Entry = Entry {
    key: "hotwheels-stunt-track-driver",
    name: "Hot Wheels Stunt Track Driver",
    image: EntryImage {
        disc_dir: "hotwheels-stunt-track-driver",
        pressing: "hotwheels-stunt-track-driver",
        path: "SETUP/HOT98/STUNT.EXE",
        in_cabinet: false,
        sha256: "a938949d705e2e06af80c22bdd76b10b52df811342e5a5b28bc23ba716903850",
        why: "this title has one pressing and its executable is on the disc unwrapped, \
              beside the data an installer would have copied from the same directory",
    },
    // The disc carries one data track and no audio tracks: this title's music
    // is in its own files, not on the disc as Red Book audio.
    red_book: None,
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
        in_cabinet: true,
        sha256: "36f77aa72b9345af2667f8e2c2395318bdbc9ac7649d378684a6bb0179eecc01",
        why: "both of this title's pressings carry the same cabinet byte for byte, so the \
              original is taken and the Masterpiece re-release buys nothing but an EA \
              registration tree, a PDF manual and menu fonts the game never reads (ADR-0006)",
    },
    // Neither pressing has an audio track: this title's music is in Audio.pac
    // and Ambient.pac, inside its own cabinet.
    red_book: None,
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
        in_cabinet: false,
        sha256: "5cf57c284973fce9d14f5677a2e4435fd989c5e938970764d00c8932ed5128ca",
        why: "this title has one pressing and its executable is on the disc unwrapped, beside \
              the engine and data its Wise installer copies from the same directory; the \
              settings launcher LEGOIsle.exe, which only starts this one, is not run",
    },
    // One data track and no audio tracks: this title's music is in its own
    // scripts and in MIDI it plays itself.
    red_book: None,
};

/// The roster's titles that have a crate, in the order they were taken: the
/// three v1 titles and LEGO Island. A title joins when its own crate does, so
/// the launcher never offers something that cannot be started.
pub const ROSTER: &[&Entry] = &[&ROCK_RAIDERS, &HOT_WHEELS, &ALPHA_TEAM, &LEGO_ISLAND];

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
