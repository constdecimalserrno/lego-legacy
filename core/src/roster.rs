//! Generic disc/image lookup and verification helpers.
//!
//! Strictly Golden-Rule-4 compliant: zero concrete titles, pressings, or
//! sound banks live in `core`. Each title crate declares its own [`Title`],
//! and `crates/catalog` collects them for consumers such as the launcher.

use crate::title::{EntryImage, Title};

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
pub fn look_for(title: &Title) -> Found {
    let path = crate::disc::pressing_path(title.image.disc_dir, title.image.pressing);
    let mut iso = match crate::disc::Iso::open(&path) {
        Ok(iso) => iso,
        Err(crate::disc::DiscError::Missing(_)) => {
            return Found::Absent {
                looked_at: path.display().to_string(),
            };
        }
        Err(e) => return Found::Unreadable { why: e.to_string() },
    };
    match title.image.read(&mut iso) {
        Ok(bytes) => judge(&title.image, &bytes),
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
