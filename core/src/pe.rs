//! Loading a title's own binaries.
//!
//! A PE image is mapped into guest memory the way Windows would map it: its
//! sections at their own addresses, its relocations applied if it did not land
//! where it asked to, and its imports pointed at the boundary (ADR-0002).

use crate::guest::{Arena, Fault, GuestMem};
use std::fmt;

/// What the image is not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PeError {
    NotPe(&'static str),
    /// A machine this emulator does not run.
    WrongMachine(u16),
    /// The image is a copy-protection wrapper around the real one
    /// (ADR-0006). Named, never run.
    Wrapped {
        scheme: &'static str,
    },
    Truncated(&'static str),
    Fault(Fault),
}

impl fmt::Display for PeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPe(why) => write!(f, "not a PE image: {why}"),
            Self::WrongMachine(m) => write!(f, "image is for machine {m:#06x}, not x86-32"),
            Self::Wrapped { scheme } => write!(
                f,
                "this image is wrapped in {scheme} copy protection. The emulator names a \
                 wrapped image and never runs, decrypts or bypasses it; run the pressing whose \
                 executable ships unwrapped instead"
            ),
            Self::Truncated(what) => write!(f, "image ends inside its {what}"),
            Self::Fault(fault) => write!(f, "{fault}"),
        }
    }
}

impl std::error::Error for PeError {}

/// One name a loaded image imports.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Import {
    pub dll: String,
    /// The imported name, or the ordinal's own spelling when it has no name.
    pub name: String,
    /// Where the image expects the address to be written.
    pub iat: u32,
}

/// One section of a mapped image.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Section {
    pub name: String,
    pub address: u32,
    pub size: u32,
}

/// An image mapped into the guest's memory.
#[derive(Clone, Debug)]
pub struct Image {
    /// Where it actually landed.
    pub base: u32,
    /// Where it asked to land.
    pub preferred_base: u32,
    pub size: u32,
    pub entry: u32,
    pub sections: Vec<Section>,
    pub imports: Vec<Import>,
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes([*b.get(at)?, *b.get(at + 1)?]))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes([
        *b.get(at)?,
        *b.get(at + 1)?,
        *b.get(at + 2)?,
        *b.get(at + 3)?,
    ]))
}

/// Whether an image is a copy-protection wrapper rather than the game.
///
/// SafeDisc v1 splits the executable in two: a small loader keeping its code in
/// sections called `.txt` and `.txt2`, and the real program encrypted beside it
/// in a `.icd` file. Both halves are on the disc, and only the wrapper is a PE
/// the machine could execute.
#[must_use]
pub fn wrapper_scheme(sections: &[Section]) -> Option<&'static str> {
    let has = |n: &str| sections.iter().any(|s| s.name == n);
    if has(".txt") && has(".txt2") {
        return Some("SafeDisc");
    }
    None
}

/// Read an image's headers without mapping it.
///
/// # Errors
/// When the bytes are not a 32-bit PE image.
pub fn inspect(bytes: &[u8]) -> Result<Image, PeError> {
    if bytes.get(..2) != Some(b"MZ") {
        return Err(PeError::NotPe("no MZ signature"));
    }
    let pe_at = u32_at(bytes, 0x3c).ok_or(PeError::Truncated("DOS header"))? as usize;
    if bytes.get(pe_at..pe_at + 4) != Some(b"PE\0\0") {
        return Err(PeError::NotPe(
            "no PE signature where the DOS header points",
        ));
    }
    let machine = u16_at(bytes, pe_at + 4).ok_or(PeError::Truncated("COFF header"))?;
    if machine != 0x014c {
        return Err(PeError::WrongMachine(machine));
    }
    let section_count = u16_at(bytes, pe_at + 6).ok_or(PeError::Truncated("COFF header"))?;
    let optional_size = u16_at(bytes, pe_at + 20).ok_or(PeError::Truncated("COFF header"))?;
    let optional = pe_at + 24;
    if u16_at(bytes, optional) != Some(0x010b) {
        return Err(PeError::NotPe("not a 32-bit optional header"));
    }
    let entry = u32_at(bytes, optional + 16).ok_or(PeError::Truncated("optional header"))?;
    let preferred_base =
        u32_at(bytes, optional + 28).ok_or(PeError::Truncated("optional header"))?;
    let size = u32_at(bytes, optional + 56).ok_or(PeError::Truncated("optional header"))?;

    let table = optional + optional_size as usize;
    let mut sections = Vec::with_capacity(section_count as usize);
    for i in 0..section_count as usize {
        let at = table + i * 40;
        let raw = bytes
            .get(at..at + 8)
            .ok_or(PeError::Truncated("section table"))?;
        let name = String::from_utf8_lossy(raw)
            .trim_end_matches('\0')
            .to_owned();
        sections.push(Section {
            name,
            address: u32_at(bytes, at + 12).ok_or(PeError::Truncated("section table"))?,
            size: u32_at(bytes, at + 8).ok_or(PeError::Truncated("section table"))?,
        });
    }

    Ok(Image {
        base: preferred_base,
        preferred_base,
        size,
        entry,
        sections,
        imports: Vec::new(),
    })
}

/// Map an image into the arena and read its imports.
///
/// The image is placed at the address it asks for. Relocating it elsewhere is
/// what [`relocate`] is for, and no title on the roster has needed it yet.
///
/// # Errors
/// When the bytes are not a 32-bit PE image, when the image is a copy
/// protection wrapper, or when it will not fit in the arena.
pub fn load(bytes: &[u8], mem: &mut Arena) -> Result<Image, PeError> {
    let mut image = inspect(bytes)?;
    if let Some(scheme) = wrapper_scheme(&image.sections) {
        return Err(PeError::Wrapped { scheme });
    }

    let pe_at = u32_at(bytes, 0x3c).unwrap_or(0) as usize;
    let optional = pe_at + 24;
    let optional_size = u16_at(bytes, pe_at + 20).unwrap_or(0) as usize;
    let section_count = u16_at(bytes, pe_at + 6).unwrap_or(0) as usize;
    let headers_size = u32_at(bytes, optional + 60).unwrap_or(0);

    // The headers themselves are mapped: a guest reads its own module's
    // headers to find its resources, and one that walks them must find them.
    mem.write(
        image.base,
        bytes.get(..headers_size as usize).unwrap_or(bytes),
    )
    .map_err(PeError::Fault)?;

    let table = optional + optional_size;
    for i in 0..section_count {
        let at = table + i * 40;
        let virtual_size = u32_at(bytes, at + 8).unwrap_or(0);
        let virtual_address = u32_at(bytes, at + 12).unwrap_or(0);
        let raw_size = u32_at(bytes, at + 16).unwrap_or(0);
        let raw_pointer = u32_at(bytes, at + 20).unwrap_or(0) as usize;
        // A section's file image can be shorter than its address space; the
        // remainder is zeroes, which the arena already holds.
        let take = raw_size.min(virtual_size.max(raw_size)) as usize;
        let raw = bytes
            .get(raw_pointer..raw_pointer.saturating_add(take))
            .unwrap_or(&[]);
        if !raw.is_empty() {
            mem.write(image.base.wrapping_add(virtual_address), raw)
                .map_err(PeError::Fault)?;
        }
    }

    image.imports = read_imports(bytes, image.base, optional)?;
    Ok(image)
}

/// The names an image imports, in the order its own tables give them.
fn read_imports(bytes: &[u8], base: u32, optional: usize) -> Result<Vec<Import>, PeError> {
    let directory = u32_at(bytes, optional + 104).unwrap_or(0);
    if directory == 0 {
        return Ok(Vec::new());
    }
    // Reading from the file image, so every address has to come back through
    // the section table to a file offset.
    let to_file = |rva: u32| -> Option<usize> {
        let pe_at = u32_at(bytes, 0x3c)? as usize;
        let optional_size = u16_at(bytes, pe_at + 20)? as usize;
        let count = u16_at(bytes, pe_at + 6)? as usize;
        let table = pe_at + 24 + optional_size;
        for i in 0..count {
            let at = table + i * 40;
            let va = u32_at(bytes, at + 12)?;
            let vsize = u32_at(bytes, at + 8)?;
            let raw_size = u32_at(bytes, at + 16)?;
            let raw = u32_at(bytes, at + 20)?;
            let span = vsize.max(raw_size);
            if rva >= va && rva < va.saturating_add(span) {
                return Some((raw + (rva - va)) as usize);
            }
        }
        None
    };

    let mut imports = Vec::new();
    let mut descriptor = to_file(directory).ok_or(PeError::Truncated("import directory"))?;
    loop {
        let lookup = u32_at(bytes, descriptor).unwrap_or(0);
        let name_rva = u32_at(bytes, descriptor + 12).unwrap_or(0);
        let iat_rva = u32_at(bytes, descriptor + 16).unwrap_or(0);
        if name_rva == 0 && iat_rva == 0 {
            break;
        }
        let dll = to_file(name_rva)
            .map(|at| {
                let end = bytes[at..].iter().position(|b| *b == 0).unwrap_or(0) + at;
                String::from_utf8_lossy(&bytes[at..end]).into_owned()
            })
            .unwrap_or_default();

        // The lookup table names what is imported; the address table is where
        // the answers go. An image with no lookup table uses the address table
        // for both.
        let names_rva = if lookup == 0 { iat_rva } else { lookup };
        let mut slot = 0u32;
        while let Some(at) = to_file(names_rva + slot * 4) {
            let value = u32_at(bytes, at).unwrap_or(0);
            if value == 0 {
                break;
            }
            let name = if value & 0x8000_0000 != 0 {
                format!("#{}", value & 0xffff)
            } else {
                to_file(value)
                    .map(|hint| {
                        let start = hint + 2;
                        let end = bytes[start..].iter().position(|b| *b == 0).unwrap_or(0) + start;
                        String::from_utf8_lossy(&bytes[start..end]).to_string()
                    })
                    .unwrap_or_default()
            };
            imports.push(Import {
                dll: dll.clone(),
                name,
                iat: base.wrapping_add(iat_rva).wrapping_add(slot * 4),
            });
            slot += 1;
        }
        descriptor += 20;
    }
    Ok(imports)
}

/// Apply an image's base relocations, for an image that did not land where it
/// asked to.
///
/// # Errors
/// When a relocation reaches outside the arena.
pub fn relocate(bytes: &[u8], image: &Image, mem: &mut Arena) -> Result<usize, PeError> {
    let delta = image.base.wrapping_sub(image.preferred_base);
    if delta == 0 {
        return Ok(0);
    }
    let pe_at = u32_at(bytes, 0x3c).unwrap_or(0) as usize;
    let optional = pe_at + 24;
    let (directory, size) = (
        u32_at(bytes, optional + 152).unwrap_or(0),
        u32_at(bytes, optional + 156).unwrap_or(0),
    );
    if directory == 0 || size == 0 {
        return Ok(0);
    }
    let mut applied = 0;
    let mut at = image.base.wrapping_add(directory);
    let end = at.wrapping_add(size);
    while at < end {
        let page = mem.read_u32(at).map_err(PeError::Fault)?;
        let block = mem.read_u32(at + 4).map_err(PeError::Fault)?;
        if block < 8 {
            break;
        }
        for i in 0..(block - 8) / 2 {
            let entry = mem.read_u32(at + 8 + i * 2).map_err(PeError::Fault)? as u16;
            let kind = entry >> 12;
            let offset = u32::from(entry & 0xfff);
            // Type 3 is the only one a 32-bit image uses; type 0 is padding.
            if kind == 3 {
                let target = image.base.wrapping_add(page).wrapping_add(offset);
                let value = mem.read_u32(target).map_err(PeError::Fault)?;
                mem.write_u32(target, value.wrapping_add(delta))
                    .map_err(PeError::Fault)?;
                applied += 1;
            }
        }
        at = at.wrapping_add(block);
    }
    Ok(applied)
}
