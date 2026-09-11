//! The declared install tree (ADR-0002's other half).
//!
//! What a title's installer would have put on the machine's disc, declared
//! rather than produced by running the installer. Every file the guest can
//! open is one the pressing carries; nothing on the host is reachable, because
//! the guest is not on the host.
//!
//! What the guest writes is the one exception, and it does not reach the host
//! either: it lands in the title's profile, a directory of its own that the
//! guest sees as part of its disc and nothing else can be read from.

use crate::disc::Iso;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// Where one of the guest's files comes from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// A file inside the pressing's own image.
    Disc(String),
    /// A file packed into the pressing's cabinet, by its index there.
    Packed(usize),
    /// Bytes the machine makes up, because the installer would have written
    /// them rather than copied them.
    Declared(Vec<u8>),
}

/// The declared machine's disc.
pub struct Tree {
    /// The pressing's own archive, and where its data lives on the disc.
    cabinet: Option<(crate::cabinet::Cabinet, String)>,
    /// Guest paths, upper-cased and with their separators normalised, to what
    /// stands behind them.
    files: BTreeMap<String, Source>,
    iso: Option<Iso>,
    /// Where what the guest writes lands, so that a save outlives the run that
    /// made it. Without one the guest's disc is read-only.
    profile: Option<PathBuf>,
    /// Every path the guest asked for and did not find, in the order it asked.
    pub missing: Vec<String>,
}

impl Default for Tree {
    fn default() -> Self {
        Self::new()
    }
}

impl Tree {
    #[must_use]
    pub fn new() -> Self {
        Self {
            cabinet: None,
            files: BTreeMap::new(),
            iso: None,
            profile: None,
            missing: Vec::new(),
        }
    }

    /// Give the tree the pressing its files come from.
    pub fn set_disc(&mut self, iso: Iso) {
        self.iso = Some(iso);
    }

    /// Give the tree somewhere to keep what the guest writes.
    pub fn set_profile(&mut self, dir: PathBuf) {
        self.profile = Some(dir);
    }

    /// Where in the profile a guest path lives: one host directory per path
    /// component, the drive letter dropped and every name as the declared
    /// machine reads it, so a file written as one spelling is found by any
    /// other. Nothing above the profile is reachable.
    fn profile_path(&self, guest_path: &str) -> Option<PathBuf> {
        let mut path = self.profile.clone()?;
        let mut parts = 0;
        for part in Self::key(guest_path).split('\\') {
            if part.is_empty() || part == "." || part == ".." || part.ends_with(':') {
                continue;
            }
            path.push(part);
            parts += 1;
        }
        (parts > 0).then_some(path)
    }

    /// Keep what the guest wrote, where the guest will read it back from.
    ///
    /// # Errors
    /// When there is no profile to write to, or the host refuses the write.
    pub fn write(&mut self, guest_path: &str, bytes: &[u8]) -> std::io::Result<PathBuf> {
        let path = self.profile_path(guest_path).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "this run has no profile to write to",
            )
        })?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&path, bytes)?;
        Ok(path)
    }

    /// Take back a file the guest wrote. What the pressing itself holds is
    /// not the guest's to delete, and saying so is honest: a title that
    /// deletes a save sees it go, and one that deletes its own data does not.
    pub fn remove(&mut self, guest_path: &str) -> bool {
        self.profile_path(guest_path)
            .is_some_and(|path| std::fs::remove_file(path).is_ok())
    }

    /// The form a guest path is looked up by: case does not matter on the
    /// declared machine, and neither does which way the separators lean.
    fn key(path: &str) -> String {
        path.trim().replace('/', "\\").to_uppercase()
    }

    /// Declare everything the pressing's cabinet holds, under a root.
    ///
    /// A packed entry becomes a file the guest can open; one the header names
    /// but does not pack sits loose on the disc, and is mapped there instead.
    ///
    /// # Errors
    /// When the header is not a cabinet.
    pub fn set_cabinet(
        &mut self,
        header: &[u8],
        data_path: &str,
        root: &str,
    ) -> Result<usize, crate::cabinet::CabinetError> {
        let cabinet = crate::cabinet::Cabinet::open(header)?;
        let mut declared = 0;
        for (index, entry) in cabinet.entries().iter().enumerate() {
            let guest = entry.guest_path(root);
            if entry.in_cabinet() {
                self.files.insert(Self::key(&guest), Source::Packed(index));
            } else {
                let on_disc = if entry.directory.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{}/{}", entry.directory.replace('\\', "/"), entry.name)
                };
                self.files.insert(Self::key(&guest), Source::Disc(on_disc));
            }
            declared += 1;
        }
        self.cabinet = Some((cabinet, data_path.to_owned()));
        Ok(declared)
    }

    /// Declare that a guest path holds one of the pressing's files.
    pub fn map(&mut self, guest_path: &str, disc_path: &str) {
        self.files
            .insert(Self::key(guest_path), Source::Disc(disc_path.to_owned()));
    }

    /// Declare a file the installer would have written rather than copied.
    pub fn declare(&mut self, guest_path: &str, bytes: Vec<u8>) {
        self.files
            .insert(Self::key(guest_path), Source::Declared(bytes));
    }

    /// Whether the tree holds a path.
    #[must_use]
    pub fn holds(&self, guest_path: &str) -> bool {
        self.files.contains_key(&Self::key(guest_path))
            || self
                .profile_path(guest_path)
                .is_some_and(|path| path.exists())
    }

    /// Read a file the guest named, recording a miss so the tree can be told
    /// about it.
    pub fn read(&mut self, guest_path: &str) -> Option<Vec<u8>> {
        let key = Self::key(guest_path);
        // What the guest wrote stands in front of what the pressing holds:
        // writing over a file is how a title keeps its settings.
        if let Some(bytes) = self.profile_path(&key).and_then(|p| std::fs::read(p).ok()) {
            return Some(bytes);
        }
        match self.files.get(&key).cloned() {
            Some(Source::Declared(bytes)) => Some(bytes),
            Some(Source::Packed(index)) => {
                let (cabinet, data_path) = self.cabinet.as_ref()?;
                let entry = cabinet.entries().get(index)?.clone();
                let iso = self.iso.as_mut()?;
                let span = iso
                    .read_range(data_path, entry.offset, entry.compressed as usize)
                    .ok()?;
                let (cabinet, _) = self.cabinet.as_ref()?;
                match cabinet.unpack(&entry, &span) {
                    Ok(bytes) => Some(bytes),
                    Err(_) => {
                        self.missing.push(guest_path.to_owned());
                        None
                    }
                }
            }
            Some(Source::Disc(path)) => {
                let iso = self.iso.as_mut()?;
                match iso.read(&path) {
                    Ok(bytes) => Some(bytes),
                    Err(_) => {
                        self.missing.push(guest_path.to_owned());
                        None
                    }
                }
            }
            None => {
                // The declared machine has the pressing in its CD drive, so a
                // path on D: is a path on the disc, spelled the way the disc
                // spells it.
                if let Some(rest) = key.strip_prefix("D:\\") {
                    let on_disc = rest.replace('\\', "/");
                    if let Some(iso) = self.iso.as_mut() {
                        if let Ok(bytes) = iso.read(&on_disc) {
                            return Some(bytes);
                        }
                    }
                }
                self.missing.push(guest_path.to_owned());
                None
            }
        }
    }

    /// How many files the tree holds.
    #[must_use]
    pub fn len(&self) -> usize {
        self.files.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }
}
