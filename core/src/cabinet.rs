//! The pressing's own archive.
//!
//! An InstallShield cabinet: a header file naming what is inside, and one or
//! more data files holding it. A title's data lives in here rather than loose
//! on the disc, so the declared install tree reads it from here.
//!
//! Two versions are on the roster. Rock Raiders ships version 5, whose header
//! ends in a table of per-file offsets. Alpha Team ships version 6, whose file
//! descriptors are a flat array and whose data is split across two volumes
//! (#52). They share the common header, the cabinet descriptor and the way the
//! bytes are compressed; everything between differs.
//!
//! Everything below was measured against the pressings themselves, because the
//! format has no specification anyone published.

use crate::inflate::inflate;
use crate::pe::{u16_at, u32_at};
use std::fmt;

/// What the header turned out not to be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CabinetError {
    NotACabinet,
    /// A cabinet of a version this reader has not been taught.
    UnknownVersion(u32),
    Truncated(&'static str),
    /// A size or an offset past four gigabytes, which nothing on this roster
    /// has and this reader would silently truncate.
    TooBig,
    /// An entry compressed a way this reader does not know.
    UnknownCompression {
        name: String,
        flags: u16,
    },
    /// The entry's own data would not decompress.
    Corrupt {
        name: String,
        why: String,
    },
    /// The entry is not in the cabinet at all.
    NotInCabinet {
        name: String,
    },
}

impl fmt::Display for CabinetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotACabinet => write!(f, "not an InstallShield cabinet"),
            Self::UnknownVersion(major) => write!(
                f,
                "an InstallShield {major} cabinet, and this reader has been taught 5 and 6; \
                 reading it as a version it is not would hand back wrong bytes"
            ),
            Self::Truncated(what) => write!(f, "the cabinet header ends inside its {what}"),
            Self::TooBig => write!(
                f,
                "the header carries a size or an offset past four gigabytes, which this reader \
                 would have to truncate"
            ),
            Self::UnknownCompression { name, flags } => write!(
                f,
                "{name} is stored with flags {flags:#06x}, which this reader has not been \
                 taught; handing back wrong bytes would be worse than stopping"
            ),
            Self::Corrupt { name, why } => write!(f, "{name} would not decompress: {why}"),
            Self::NotInCabinet { name } => {
                write!(f, "{name} is named by the header but is not in the cabinet")
            }
        }
    }
}

impl std::error::Error for CabinetError {}

/// One file the cabinet's header names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// The directory the installer would have put it in, as the header spells
    /// it.
    pub directory: String,
    /// How big it is once unpacked.
    pub expanded: u32,
    /// How much room it takes in the cabinet.
    pub compressed: u32,
    /// Where its data starts in the cabinet.
    pub offset: u32,
    /// Bit two says it is compressed; an entry with none of these set is not
    /// in the cabinet and sits loose on the disc instead.
    pub flags: u16,
    /// Which of the cabinet's data files holds it, counting from one. A
    /// version 5 cabinet on this roster has one; Alpha Team's has two.
    pub volume: u16,
}

impl Entry {
    /// Whether this entry's bytes are in the cabinet at all.
    #[must_use]
    pub fn in_cabinet(&self) -> bool {
        self.flags & 4 != 0 && self.offset != 0
    }

    /// The path the installer would have written it to, under a root.
    #[must_use]
    pub fn guest_path(&self, root: &str) -> String {
        if self.directory.is_empty() {
            format!("{root}\\{}", self.name)
        } else {
            format!(
                "{root}\\{}\\{}",
                self.directory.replace('/', "\\"),
                self.name
            )
        }
    }
}

/// A cabinet's header, read.
pub struct Cabinet {
    entries: Vec<Entry>,
}

/// A version 6 size or offset, which the header writes as sixty-four bits.
///
/// Nothing on this roster needs the top half — the largest entry anywhere is
/// Alpha Team's 126 MB `Ambient.pac` — but a cabinet that did would be read
/// wrong rather than refused, so the top half is checked rather than dropped.
fn u64_at(b: &[u8], at: usize) -> Option<u32> {
    let low = u32_at(b, at)?;
    if u32_at(b, at + 4)? == 0 {
        Some(low)
    } else {
        None
    }
}

/// How far apart two version 6 file descriptors are, which is how that version
/// finds them at all: they are an array rather than a table of offsets.
const DESCRIPTOR6: usize = 0x57;

impl Cabinet {
    /// Read a cabinet's header.
    ///
    /// # Errors
    /// When the bytes are not an InstallShield 5 or 6 header.
    pub fn open(header: &[u8]) -> Result<Self, CabinetError> {
        if header.get(..4) != Some(b"ISc(") {
            return Err(CabinetError::NotACabinet);
        }
        // The version word's shape: Rock Raiders' is 0x01005201 and Alpha
        // Team's is 0x0100600c, and the digit that matters is the same one in
        // both.
        let major =
            (u32_at(header, 0x04).ok_or(CabinetError::Truncated("common header"))? >> 12) & 0xf;
        let descriptor =
            u32_at(header, 0x0c).ok_or(CabinetError::Truncated("common header"))? as usize;
        let table = descriptor
            + u32_at(header, descriptor + 0x0c).ok_or(CabinetError::Truncated("descriptor"))?
                as usize;
        let directories = u32_at(header, descriptor + 0x1c)
            .ok_or(CabinetError::Truncated("descriptor"))? as usize;
        let files = u32_at(header, descriptor + 0x28)
            .ok_or(CabinetError::Truncated("descriptor"))? as usize;

        // A NUL-terminated name, at an offset measured from the table.
        let name_at = |offset: usize| -> String {
            let start = table + offset;
            let end = header[start.min(header.len())..]
                .iter()
                .position(|b| *b == 0)
                .map_or(header.len(), |n| start + n);
            String::from_utf8_lossy(header.get(start..end).unwrap_or(&[])).into_owned()
        };

        // The table is one offset per directory, then — in version 5 — one per
        // file.
        let offset_of = |index: usize| -> Option<usize> {
            u32_at(header, table + index * 4).map(|o| o as usize)
        };

        let mut entries = Vec::with_capacity(files);
        match major {
            5 => {
                for i in 0..files {
                    let Some(at) = offset_of(directories + i).map(|o| table + o) else {
                        break;
                    };
                    let Some(name_offset) = u32_at(header, at) else {
                        break;
                    };
                    let directory_index = u32_at(header, at + 4).unwrap_or(0) as usize;
                    let directory = offset_of(directory_index).map_or_else(String::new, name_at);
                    entries.push(Entry {
                        name: name_at(name_offset as usize),
                        directory,
                        flags: u16_at(header, at + 8).unwrap_or(0),
                        expanded: u32_at(header, at + 0x0a).unwrap_or(0),
                        compressed: u32_at(header, at + 0x0e).unwrap_or(0),
                        offset: u32_at(header, at + 0x26).unwrap_or(0),
                        // Every entry of the one version 5 cabinet on this
                        // roster is in DATA1.CAB, and nothing here reads a
                        // volume it was not told about.
                        volume: 1,
                    });
                }
            }
            // Version 6 drops the per-file offsets and lays the descriptors out
            // as a flat array of their own, at a second offset the cabinet
            // descriptor carries.
            6 => {
                let second = u32_at(header, descriptor + 0x2c)
                    .ok_or(CabinetError::Truncated("descriptor"))?
                    as usize;
                for i in 0..files {
                    let at = table + second + i * DESCRIPTOR6;
                    let Some(name_offset) = u32_at(header, at + 0x3a) else {
                        break;
                    };
                    let directory_index = u16_at(header, at + 0x3e).unwrap_or(0) as usize;
                    let directory = offset_of(directory_index).map_or_else(String::new, name_at);
                    entries.push(Entry {
                        name: name_at(name_offset as usize),
                        directory,
                        flags: u16_at(header, at).unwrap_or(0),
                        expanded: u64_at(header, at + 0x02).ok_or(CabinetError::TooBig)?,
                        compressed: u64_at(header, at + 0x0a).ok_or(CabinetError::TooBig)?,
                        offset: u64_at(header, at + 0x12).ok_or(CabinetError::TooBig)?,
                        volume: u16_at(header, at + 0x55).unwrap_or(1),
                    });
                }
            }
            _ => return Err(CabinetError::UnknownVersion(major)),
        }
        Ok(Self { entries })
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Find an entry by the name the header gives it.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<&Entry> {
        self.entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// Unpack one entry from the cabinet's data.
    ///
    /// The data is a run of chunks, each a two-byte length and then that many
    /// bytes of a complete DEFLATE stream. Measured, not documented: treating
    /// the whole run as one stream fails after the first chunk.
    ///
    /// `data` is the entry's own span out of the cabinet, so a run reads the
    /// bytes it needs rather than the whole archive.
    ///
    /// # Errors
    /// When the entry is not in the cabinet, is stored a way this reader does
    /// not know, or will not decompress to the size its header gives.
    pub fn unpack(&self, entry: &Entry, data: &[u8]) -> Result<Vec<u8>, CabinetError> {
        if !entry.in_cabinet() {
            return Err(CabinetError::NotInCabinet {
                name: entry.name.clone(),
            });
        }
        // A header may name a file it holds no bytes for: version 6 cabinets
        // carry placeholders the installer would have filled in, and Alpha
        // Team's has seven of them. An empty file is what they are, and that
        // is not a compression scheme to guess at.
        if entry.compressed == 0 {
            return Ok(Vec::new());
        }
        if entry.flags & !0x0004 != 0 {
            return Err(CabinetError::UnknownCompression {
                name: entry.name.clone(),
                flags: entry.flags,
            });
        }

        let mut out = Vec::with_capacity(entry.expanded as usize);
        let mut at = 0;
        let end = (entry.compressed as usize).min(data.len());
        while at < end {
            let len = u16_at(data, at).ok_or_else(|| CabinetError::Corrupt {
                name: entry.name.clone(),
                why: "the cabinet ends inside a chunk's length".to_owned(),
            })? as usize;
            at += 2;
            let chunk = data
                .get(at..at + len)
                .ok_or_else(|| CabinetError::Corrupt {
                    name: entry.name.clone(),
                    why: "the cabinet ends inside a chunk".to_owned(),
                })?;
            let bytes = inflate(chunk).map_err(|e| CabinetError::Corrupt {
                name: entry.name.clone(),
                why: e.to_string(),
            })?;
            out.extend_from_slice(&bytes);
            at += len;
        }
        // Chunks that each inflate are still damage when together they come
        // to another size than the header's (#169).
        if out.len() != entry.expanded as usize {
            return Err(CabinetError::Corrupt {
                name: entry.name.clone(),
                why: format!(
                    "it unpacks to {} bytes, and the header says {}",
                    out.len(),
                    entry.expanded
                ),
            });
        }
        Ok(out)
    }
}
