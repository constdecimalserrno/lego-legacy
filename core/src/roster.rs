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
        packed: false,
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
        packed: false,
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
        packed: true,
        sha256: "36f77aa72b9345af2667f8e2c2395318bdbc9ac7649d378684a6bb0179eecc01",
        why: "both of this title's pressings carry the same cabinet byte for byte, so the \
              original is taken and the Masterpiece re-release buys nothing but an EA \
              registration tree, a PDF manual and menu fonts the game never reads (ADR-0006)",
    },
    // Neither pressing has an audio track: this title's music is in Audio.pac
    // and Ambient.pac, inside its own cabinet.
    red_book: None,
};

/// The roster's supported titles, three of its four, in the order they were
/// taken. A title joins when its own crate does, so the launcher never offers
/// something that cannot be started.
pub const ROSTER: &[&Entry] = &[&ROCK_RAIDERS, &HOT_WHEELS, &ALPHA_TEAM];

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
        }
    }
}

/// Go looking for a title's image.
#[must_use]
pub fn look_for(entry: &Entry) -> Found {
    let path = crate::disc::pressing_path(entry.image.disc_dir, entry.image.pressing);
    let mut iso = match crate::disc::Iso::open(&path) {
        Ok(iso) => iso,
        Err(_) => {
            return Found::Absent {
                looked_at: path.display().to_string(),
            };
        }
    };
    let Ok(bytes) = entry.image.read(&mut iso) else {
        return Found::Absent {
            looked_at: format!("{}!{}", path.display(), entry.image.path),
        };
    };
    let pressing = entry.image.pressing.to_owned();
    if let Ok(image) = crate::pe::inspect(&bytes) {
        if let Some(scheme) = crate::pe::wrapper_scheme(&image.sections) {
            return Found::Wrapped {
                pressing,
                scheme: scheme.to_owned(),
            };
        }
    }
    let digest = crate::hash::sha256_hex(&bytes);
    if digest == entry.image.sha256 {
        Found::Runnable { pressing }
    } else {
        Found::WrongBytes {
            pressing,
            found: digest,
        }
    }
}
