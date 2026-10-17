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

/// Whether a name matches a pattern with `*` and `?` in it, as a guest's own
/// enumeration spells one. Both are already upper-cased by the caller.
fn matches(name: &str, pattern: &str) -> bool {
    let (n, p): (Vec<char>, Vec<char>) = (name.chars().collect(), pattern.chars().collect());
    // The usual two-pointer walk: remember where the last star was, and go
    // back to it with one more character eaten when the rest stops matching.
    let (mut i, mut j) = (0, 0);
    let (mut star, mut back) = (None, 0);
    while i < n.len() {
        if j < p.len() && (p[j] == '?' || p[j] == n[i]) {
            i += 1;
            j += 1;
        } else if j < p.len() && p[j] == '*' {
            star = Some(j);
            back = i;
            j += 1;
        } else if let Some(at) = star {
            j = at + 1;
            back += 1;
            i = back;
        } else {
            return false;
        }
    }
    p[j..].iter().all(|c| *c == '*')
}

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
    /// The pressing's own archive, and the stem its data files are named
    /// with: a cabinet's volume N lives in `<stem>N.CAB` beside the header.
    cabinet: Option<(crate::cabinet::Cabinet, String)>,
    /// Guest paths, upper-cased and with their separators normalised, to what
    /// stands behind them.
    files: BTreeMap<String, Source>,
    iso: Option<Iso>,
    /// Where what the guest writes lands, so that a save outlives the run that
    /// made it. Without one the guest's disc is read-only.
    profile: Option<PathBuf>,
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
    /// component, the drive among them, and every name as the declared
    /// machine reads it, so a file written as one spelling is found by any
    /// other and two drives never land on one file. Nothing above the profile
    /// is reachable.
    fn profile_path(&self, guest_path: &str) -> Option<PathBuf> {
        let mut path = self.profile.clone()?;
        let mut parts = 0;
        for part in Self::key(guest_path).split('\\') {
            let part = part.trim_end_matches(':');
            if part.is_empty() || part == "." || part == ".." {
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
    /// `data_stem` names the cabinet's data files: volume N is `<stem>N.CAB`,
    /// which is what both pressings on the roster that carry a cabinet do.
    /// Rock Raiders has one volume and Alpha Team has two.
    ///
    /// # Errors
    /// When the header is not a cabinet.
    pub fn set_cabinet(
        &mut self,
        header: &[u8],
        data_stem: &str,
        root: &str,
    ) -> Result<usize, crate::cabinet::CabinetError> {
        let cabinet = crate::cabinet::Cabinet::open(header)?;
        let mut declared = 0;
        for (index, entry) in cabinet.entries().iter().enumerate() {
            let guest = entry.guest_path(root);
            // A header may name the same file twice, once per install size:
            // Alpha Team's carries `LMap.pac` as 53 MB of hi-res lightmaps for
            // a Typical install and 3.4 MB of low-res for a Minimum one. The
            // declared machine had the Typical install — the one the
            // installer offers first and a player takes — so the larger of
            // two entries with one name wins.
            if let Some(Source::Packed(before)) = self.files.get(&Self::key(&guest)) {
                if cabinet.entries()[*before].expanded >= entry.expanded {
                    continue;
                }
            }
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
        self.cabinet = Some((cabinet, data_stem.to_owned()));
        Ok(declared)
    }

    /// Declare everything a directory of the pressing holds, under a root:
    /// what an installer that copies files rather than unpacking a cabinet
    /// would have put on the machine's disc.
    ///
    /// # Errors
    /// When the image holds no such directory, or cannot be read.
    pub fn set_copied(
        &mut self,
        iso: &mut Iso,
        from: &str,
        root: &str,
    ) -> Result<usize, crate::disc::DiscError> {
        let mut declared = 0;
        let mut dirs = vec![String::new()];
        while let Some(under) = dirs.pop() {
            let here = if under.is_empty() {
                from.to_owned()
            } else {
                format!("{from}/{under}")
            };
            for entry in iso.list(&here)? {
                let below = if under.is_empty() {
                    entry.name.clone()
                } else {
                    format!("{under}/{}", entry.name)
                };
                if entry.is_dir {
                    dirs.push(below);
                } else {
                    let guest = format!("{root}\\{}", below.replace('/', "\\"));
                    self.files.insert(
                        Self::key(&guest),
                        Source::Disc(format!("{here}/{}", entry.name)),
                    );
                    declared += 1;
                }
            }
        }
        Ok(declared)
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

    /// Whether [`Tree::read`] would find a file, answered without reading it:
    /// a packed entry is there whether or not anyone unpacks it.
    pub fn readable(&mut self, guest_path: &str) -> bool {
        let key = Self::key(guest_path);
        if self.profile_path(&key).is_some_and(|p| p.is_file()) {
            return true;
        }
        let on_disc = match self.files.get(&key) {
            Some(Source::Declared(_) | Source::Packed(_)) => return true,
            Some(Source::Disc(path)) => path.clone(),
            None => match key.strip_prefix("D:\\") {
                Some(rest) => rest.replace('\\', "/"),
                None => return false,
            },
        };
        self.iso
            .as_mut()
            .and_then(|iso| iso.entry(&on_disc).ok())
            .is_some_and(|entry| !entry.is_dir)
    }

    /// Read a file the guest named.
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
                let (cabinet, data_stem) = self.cabinet.as_ref()?;
                let entry = cabinet.entries().get(index)?.clone();
                // An entry names the volume it is in, and the volume names the
                // file: Alpha Team's game is in DATA2.CAB and only its
                // installer is in DATA1.CAB.
                let data_path = format!("{data_stem}{}.CAB", entry.volume);
                let iso = self.iso.as_mut()?;
                let span = iso
                    .read_range(&data_path, entry.offset, entry.compressed as usize)
                    .ok()?;
                let (cabinet, _) = self.cabinet.as_ref()?;
                cabinet.unpack(&entry, &span).ok()
            }
            Some(Source::Disc(path)) => self.iso.as_mut()?.read(&path).ok(),
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
                None
            }
        }
    }

    /// Every file the tree holds under a directory whose name matches a
    /// pattern, with how big each one is.
    ///
    /// What `FindFirstFileA` enumerates. The sizes come from the header rather
    /// than from unpacking anything: a title looking for its archives should
    /// not cost four hundred megabytes of inflating to answer.
    #[must_use]
    pub fn matching(&self, dir: &str, pattern: &str) -> Vec<(String, u32)> {
        let dir = Self::key(dir);
        let prefix = if dir.ends_with('\\') {
            dir
        } else {
            format!("{dir}\\")
        };
        let mut out = Vec::new();
        for (path, source) in &self.files {
            let Some(name) = path.strip_prefix(&prefix) else {
                continue;
            };
            if name.contains('\\') || !matches(name, &Self::key(pattern)) {
                continue;
            }
            let size = match source {
                Source::Declared(bytes) => u32::try_from(bytes.len()).unwrap_or(0),
                Source::Packed(index) => self
                    .cabinet
                    .as_ref()
                    .and_then(|(c, _)| c.entries().get(*index))
                    .map_or(0, |e| e.expanded),
                Source::Disc(_) => 0,
            };
            out.push((name.to_owned(), size));
        }
        out
    }
}
