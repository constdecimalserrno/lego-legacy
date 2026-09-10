//! Reading a pressing.
//!
//! A rip is an ISO-9660 image beside the cue sheet that describes the whole
//! disc. Nothing here is ever written: the rip stays exactly as the player made
//! it, and everything the guest may change lives in the profile instead.

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// One sector of an ISO-9660 image.
const SECTOR: u64 = 2048;

/// Where the volume descriptors start.
const FIRST_DESCRIPTOR: u64 = 16;

/// What went wrong reaching a pressing.
#[derive(Debug)]
pub enum DiscError {
    /// No rip where one was expected. Names the path so the answer is
    /// actionable rather than mysterious.
    Missing(PathBuf),
    /// The file is not an ISO-9660 image.
    NotAnIso(PathBuf),
    /// The image has no such file.
    NoSuchFile {
        iso: PathBuf,
        path: String,
    },
    Io(std::io::Error),
}

impl std::fmt::Display for DiscError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Missing(p) => write!(
                f,
                "no rip at {}: the emulator carries no game data, so a pressing has to be \
                 ripped from your own disc",
                p.display()
            ),
            Self::NotAnIso(p) => write!(f, "{} is not an ISO-9660 image", p.display()),
            Self::NoSuchFile { iso, path } => {
                write!(f, "{} holds no {path}", iso.display())
            }
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for DiscError {}

impl From<std::io::Error> for DiscError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

/// One name in a directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    /// The name with its version suffix removed: `LEGORR.EXE`, not
    /// `LEGORR.EXE;1`.
    pub name: String,
    pub sector: u32,
    pub len: u32,
    pub is_dir: bool,
}

/// A pressing's data track, open for reading.
pub struct Iso {
    file: File,
    path: PathBuf,
    root: (u32, u32),
    /// The volume's own name for itself.
    pub label: String,
}

impl Iso {
    /// Open an image and read its primary volume descriptor.
    ///
    /// # Errors
    /// When the file is absent, unreadable, or not an ISO-9660 image.
    pub fn open(path: &Path) -> Result<Self, DiscError> {
        if !path.is_file() {
            return Err(DiscError::Missing(path.to_path_buf()));
        }
        let mut file = File::open(path)?;
        let mut pvd = [0u8; SECTOR as usize];
        file.seek(SeekFrom::Start(FIRST_DESCRIPTOR * SECTOR))?;
        file.read_exact(&mut pvd)?;
        if pvd[0] != 1 || &pvd[1..6] != b"CD001" {
            return Err(DiscError::NotAnIso(path.to_path_buf()));
        }
        let record = &pvd[156..190];
        let root = (le32(&record[2..6]), le32(&record[10..14]));
        let label = String::from_utf8_lossy(&pvd[40..72]).trim().to_owned();
        Ok(Self {
            file,
            path: path.to_path_buf(),
            root,
            label,
        })
    }

    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What a directory holds, in the order the image records it.
    ///
    /// The order is the image's own and is passed through untouched: a guest
    /// that enumerates a directory must see the same sequence on every run,
    /// and the image is the only place that order can honestly come from.
    ///
    /// # Errors
    /// When the image cannot be read.
    pub fn list(&mut self, dir: &str) -> Result<Vec<Entry>, DiscError> {
        let (sector, len) = self.locate_dir(dir)?;
        self.read_dir(sector, len)
    }

    /// Read a whole file out of the image.
    ///
    /// # Errors
    /// When the image holds no such file.
    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, DiscError> {
        let entry = self.entry(path)?;
        if entry.is_dir {
            return Err(DiscError::NoSuchFile {
                iso: self.path.clone(),
                path: path.to_owned(),
            });
        }
        let mut bytes = vec![0u8; entry.len as usize];
        self.file
            .seek(SeekFrom::Start(u64::from(entry.sector) * SECTOR))?;
        self.file.read_exact(&mut bytes)?;
        Ok(bytes)
    }

    /// Whether the image holds this path.
    ///
    /// # Errors
    /// When the image cannot be read at all.
    pub fn has(&mut self, path: &str) -> Result<bool, DiscError> {
        match self.entry(path) {
            Ok(_) => Ok(true),
            Err(DiscError::NoSuchFile { .. }) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// One named entry, found by walking the directories on the way to it.
    ///
    /// # Errors
    /// When no such entry exists.
    pub fn entry(&mut self, path: &str) -> Result<Entry, DiscError> {
        let mut parts = path.split('/').filter(|p| !p.is_empty()).peekable();
        let mut here = self.root;
        let mut found = None;
        while let Some(part) = parts.next() {
            let entries = self.read_dir(here.0, here.1)?;
            let hit = entries
                .into_iter()
                .find(|e| e.name.eq_ignore_ascii_case(part))
                .ok_or_else(|| DiscError::NoSuchFile {
                    iso: self.path.clone(),
                    path: path.to_owned(),
                })?;
            if parts.peek().is_some() {
                here = (hit.sector, hit.len);
            } else {
                found = Some(hit);
            }
        }
        found.ok_or_else(|| DiscError::NoSuchFile {
            iso: self.path.clone(),
            path: path.to_owned(),
        })
    }

    fn locate_dir(&mut self, dir: &str) -> Result<(u32, u32), DiscError> {
        if dir.trim_matches('/').is_empty() {
            return Ok(self.root);
        }
        let entry = self.entry(dir)?;
        Ok((entry.sector, entry.len))
    }

    fn read_dir(&mut self, sector: u32, len: u32) -> Result<Vec<Entry>, DiscError> {
        let mut data = vec![0u8; len as usize];
        self.file
            .seek(SeekFrom::Start(u64::from(sector) * SECTOR))?;
        self.file.read_exact(&mut data)?;

        let mut out = Vec::new();
        let mut at = 0usize;
        while at < data.len() {
            let record_len = data[at] as usize;
            if record_len == 0 {
                // A directory's records never straddle a sector: the rest of
                // this one is padding.
                at = (at / SECTOR as usize + 1) * SECTOR as usize;
                continue;
            }
            if at + record_len > data.len() {
                break;
            }
            let record = &data[at..at + record_len];
            let name_len = record[32] as usize;
            let raw = &record[33..33 + name_len];
            // The first two entries of every directory are itself and its
            // parent, spelled as a single zero or one byte.
            if raw != b"\0" && raw != b"\x01" {
                let name = String::from_utf8_lossy(raw);
                let name = name.split(';').next().unwrap_or(&name).to_owned();
                out.push(Entry {
                    name,
                    sector: le32(&record[2..6]),
                    len: le32(&record[10..14]),
                    is_dir: record[25] & 2 != 0,
                });
            }
            at += record_len;
        }
        Ok(out)
    }
}

fn le32(b: &[u8]) -> u32 {
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

/// Where a title's pressings are ripped to.
///
/// `LEGO_LEGACY_GAME_DATA` moves the root; the default is `game-data` beside
/// the working directory, which is where the rips live in a checkout.
#[must_use]
pub fn game_data_root() -> PathBuf {
    std::env::var_os("LEGO_LEGACY_GAME_DATA")
        .map_or_else(|| PathBuf::from("game-data"), PathBuf::from)
}

/// The image of one pressing: `<root>/<title>/<pressing>/<pressing>.iso`.
#[must_use]
pub fn pressing_path(title: &str, pressing: &str) -> PathBuf {
    game_data_root()
        .join(title)
        .join(pressing)
        .join(format!("{pressing}.iso"))
}

/// Open one pressing of a title.
///
/// # Errors
/// When the rip is absent or unreadable.
pub fn open_pressing(title: &str, pressing: &str) -> Result<Iso, DiscError> {
    Iso::open(&pressing_path(title, pressing))
}
