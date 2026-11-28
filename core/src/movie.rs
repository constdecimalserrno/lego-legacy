//! Shared movie asset cache and Indeo 5 playback engine.
//!
//! - [`MovieStore`] deduplicates immutable movie file bytes (`Rc<[u8]>`) by
//!   canonicalized upper-case guest path, resolves files during `LLSNAP v2`
//!   restore, and evicts unreferenced entries.
//! - [`MoviePlayer`] drives declared-clock Indeo 5 (`IV50`) video stepping,
//!   [`Mixer`] PCM audio sync, RGB565 surface blitting, and key-rehydrated
//!   `LLSNAP v2` serialization (omitting raw AVI bytes).

use crate::avi::{self, Avi};
use crate::guest::{Arena, Fault, GuestMem};
use crate::install::Tree;
use crate::iv50::{self, Decoder, Picture};
use crate::snap::{Snap, SnapError, Visitor};
use crate::sound::{self, Mixer};
use std::collections::BTreeMap;
use std::rc::Rc;

/// Canonicalize a guest path identically to `Tree::key`: upper-case,
/// backslash separators, and collapsed `.`/`..`/empty segments.
fn normalize_key(path: &str) -> String {
    let norm = path.trim().replace('/', "\\").to_uppercase();
    let (prefix, rest) = if norm.as_bytes().get(1) == Some(&b':') {
        (&norm[..2], norm[2..].trim_start_matches('\\'))
    } else if !norm.contains('\\') {
        return norm;
    } else {
        ("", norm.as_str())
    };
    let mut parts: Vec<&str> = Vec::new();
    for seg in rest.split('\\') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    if prefix.is_empty() {
        parts.join("\\")
    } else if parts.is_empty() {
        format!("{prefix}\\")
    } else {
        format!("{prefix}\\{}", parts.join("\\"))
    }
}

/// Deduplicated cache of immutable movie file bytes (`Rc<[u8]>`), keyed by
/// canonicalized upper-case guest path.
#[derive(Default)]
pub struct MovieStore {
    entries: BTreeMap<String, Rc<[u8]>>,
}

impl MovieStore {
    /// Number of movie files currently retained in the cache.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache holds no movie files.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Whether `path` (canonicalized) is currently cached.
    #[must_use]
    pub fn contains(&self, path: &str) -> bool {
        self.entries.contains_key(&normalize_key(path))
    }

    /// Borrow the cached `Rc<[u8]>` for `path` (canonicalized), if present.
    #[must_use]
    pub fn get(&self, path: &str) -> Option<&Rc<[u8]>> {
        self.entries.get(&normalize_key(path))
    }

    /// Load `abs_path` from the cache or `tree`, returning `(canonical_key, bytes)`.
    pub fn load(&mut self, abs_path: &str, tree: &mut Tree) -> Option<(String, Rc<[u8]>)> {
        let key = normalize_key(abs_path);
        if key.is_empty() {
            return None;
        }
        if let Some(cached) = self.entries.get(&key) {
            return Some((key, cached.clone()));
        }
        let loaded = tree.read(abs_path)?;
        let rc: Rc<[u8]> = loaded.into();
        self.entries.insert(key.clone(), rc.clone());
        Some((key, rc))
    }

    /// Resolve immutable movie bytes during `LLSNAP v2` snapshot restore,
    /// hitting the live cache first and falling back to `tree`.
    ///
    /// # Errors
    /// Returns [`SnapError::Corrupt`] if `key` is empty or missing from `tree`.
    pub fn resolve_for_restore(
        &mut self,
        key: &str,
        tree: &mut Tree,
    ) -> Result<Rc<[u8]>, SnapError> {
        let norm = normalize_key(key);
        if norm.is_empty() {
            return Err(SnapError::Corrupt("movie key missing"));
        }
        self.load(&norm, tree)
            .map(|(_, bytes)| bytes)
            .ok_or(SnapError::Corrupt("movie missing from tree"))
    }

    /// Drop every cached entry with no live holder outside this store.
    pub fn evict_unused(&mut self) {
        self.entries.retain(|_, rc| Rc::strong_count(rc) > 1);
    }
}

/// Validate that every chunk range in `avi` fits within `bytes_len`.
pub(crate) fn validate_chunk_bounds(avi: &Avi, bytes_len: usize) -> Result<(), SnapError> {
    for stream in &avi.streams {
        for &(at, len) in &stream.chunks {
            if at.checked_add(len).is_none_or(|end| end > bytes_len) {
                return Err(SnapError::Corrupt("movie chunk out of bounds"));
            }
        }
    }
    Ok(())
}

/// Why an AVI could not be opened as an IV50 [`MoviePlayer`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpenError {
    /// File not present in the declared install tree.
    Missing,
    /// RIFF AVI container failed to parse.
    Parse(avi::Error),
    /// AVI container carries no video stream.
    NoVideo,
    /// Video stream uses a codec other than `IV50`.
    UnsupportedCodec([u8; 4]),
}

/// Outcome of calling [`MoviePlayer::start`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StartOutcome {
    /// Transitioned from stopped to running (audio started on `mixer` if present).
    Started,
    /// Already running; playback state and mixer left unchanged.
    AlreadyRunning,
    /// Already at end-of-stream (`next >= frames`); `first_eos` is true when
    /// this call was the first to observe completion.
    AtEnd { first_eos: bool },
}

/// Why [`MoviePlayer::step_rgb565`] failed.
#[derive(Debug)]
pub enum StepError {
    /// Indeo 5 bitstream decode failure on frame `index`.
    Decode { index: usize, error: iv50::Error },
    /// Guest memory fault while blitting the decoded RGB565 frame.
    Fault(Fault),
}

/// Declared-clock Indeo 5 movie player shared by `amstream` and `dshow`.
pub struct MoviePlayer {
    name: String,
    key: String,
    bytes: Rc<[u8]>,
    avi: Avi,
    video: usize,
    decoder: Decoder,
    picture: Picture,
    next: usize,
    running_since: Option<u64>,
    completed: bool,
}

impl MoviePlayer {
    /// Load `abs_path` through `store`/`tree`, parse its AVI container, and
    /// verify it carries an `IV50` video stream.
    ///
    /// # Errors
    /// Returns [`OpenError`] if the file is missing, unparseable, lacks video,
    /// or uses a non-IV50 video codec. Any freshly cached bytes are evicted on
    /// failure.
    pub fn open(
        name: String,
        abs_path: &str,
        store: &mut MovieStore,
        tree: &mut Tree,
    ) -> Result<Self, OpenError> {
        let Some((key, bytes)) = store.load(abs_path, tree) else {
            return Err(OpenError::Missing);
        };
        let parsed = (|| {
            let avi = avi::parse(&bytes).map_err(OpenError::Parse)?;
            let video = avi
                .streams
                .iter()
                .position(avi::Stream::is_video)
                .ok_or(OpenError::NoVideo)?;
            let codec = avi.streams[video].compression();
            if !codec.eq_ignore_ascii_case(b"IV50") {
                return Err(OpenError::UnsupportedCodec(codec));
            }
            Ok((avi, video))
        })();
        match parsed {
            Ok((avi, video)) => Ok(Self {
                name,
                key,
                bytes,
                avi,
                video,
                decoder: Decoder::new(),
                picture: Picture::default(),
                next: 0,
                running_since: None,
                completed: false,
            }),
            Err(err) => {
                drop(bytes);
                store.evict_unused();
                Err(err)
            }
        }
    }

    pub(crate) fn empty_for_snap() -> Self {
        Self {
            name: String::new(),
            key: String::new(),
            bytes: Rc::from([]),
            avi: Avi::default(),
            video: 0,
            decoder: Decoder::new(),
            picture: Picture::default(),
            next: 0,
            running_since: None,
            completed: false,
        }
    }

    /// Rehydrate immutable AVI bytes from `store`/`tree` after an `LLSNAP v2`
    /// restore, verifying all chunk ranges against the loaded buffer.
    ///
    /// # Errors
    /// Returns [`SnapError::Corrupt`] when the file is missing or truncated.
    pub fn rehydrate(&mut self, store: &mut MovieStore, tree: &mut Tree) -> Result<(), SnapError> {
        if self.bytes.is_empty() {
            let bytes = store.resolve_for_restore(&self.key, tree)?;
            if let Err(err) = validate_chunk_bounds(&self.avi, bytes.len()) {
                drop(bytes);
                store.evict_unused();
                return Err(err);
            }
            self.bytes = bytes;
        }
        Ok(())
    }

    /// Guest-supplied movie path used in logs and walls.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Video picture dimensions `(width, height)` from the AVI `strf` header.
    #[must_use]
    pub fn picture_size(&self) -> (u32, u32) {
        self.avi.streams[self.video].picture_size()
    }

    /// Total number of indexed video chunks.
    #[must_use]
    pub fn frames(&self) -> usize {
        self.avi.streams[self.video].chunks.len()
    }

    /// Index of the next video chunk to decode.
    #[must_use]
    pub fn next_frame(&self) -> usize {
        self.next
    }

    /// Whether at least one undecoded video chunk remains.
    #[must_use]
    pub fn has_next(&self) -> bool {
        self.next < self.frames()
    }

    /// Declared-clock start anchor in milliseconds while running, or `None` while stopped.
    #[must_use]
    pub fn running_since(&self) -> Option<u64> {
        self.running_since
    }

    /// Whether playback is currently running and has not yet completed.
    #[must_use]
    pub fn is_running(&self) -> bool {
        self.running_since.is_some() && !self.completed
    }

    /// Mark the current playthrough as completed (for `EC_COMPLETE` suppression).
    pub fn mark_completed(&mut self) {
        self.completed = true;
    }

    /// When frame `n` is due, in declared milliseconds from stream start.
    fn due_millis(&self, n: usize) -> u64 {
        let (num, den) = self.avi.streams[self.video].frame_millis();
        let ms = u128::from(n as u64) * u128::from(num) / u128::from(den);
        u64::try_from(ms).unwrap_or(u64::MAX)
    }

    /// Total stream duration in declared milliseconds.
    #[must_use]
    pub fn duration_millis(&self) -> u64 {
        self.due_millis(self.frames())
    }

    /// Total stream duration in seconds (`IMediaPosition::get_Duration`).
    #[must_use]
    pub fn duration_secs(&self) -> f64 {
        let stream = &self.avi.streams[self.video];
        (self.frames() as f64 * f64::from(stream.scale)) / f64::from(stream.rate.max(1))
    }

    /// Declared milliseconds elapsed since playback started, or `0` while stopped.
    #[must_use]
    pub fn elapsed_millis(&self, now_ms: u64) -> u64 {
        self.running_since
            .map_or(0, |since| now_ms.saturating_sub(since))
    }

    /// Current playback position in seconds, clamped to [`Self::duration_secs`].
    #[must_use]
    pub fn position_secs(&self, now_ms: u64) -> f64 {
        let duration = self.duration_secs();
        let secs = match self.running_since {
            Some(since) => now_ms.saturating_sub(since) as f64 / 1000.0,
            None => self.due_millis(self.next) as f64 / 1000.0,
        };
        secs.min(duration)
    }

    /// Frame index corresponding to `secs`, clamped to `0..=frames()`.
    #[must_use]
    pub fn frame_for_secs(&self, secs: f64) -> usize {
        let stream = &self.avi.streams[self.video];
        if stream.scale == 0 {
            0
        } else {
            ((secs.max(0.0) * f64::from(stream.rate.max(1))) / f64::from(stream.scale))
                .floor()
                .clamp(0.0, self.frames() as f64) as usize
        }
    }

    /// Declared-clock deadline in milliseconds for the next frame (compensated
    /// by `refresh_compensation_ms` when flipping visibly), or for stream end
    /// when all frames have been decoded. Returns `None` while stopped.
    #[must_use]
    pub fn next_deadline_millis(&self, refresh_compensation_ms: u64) -> Option<u64> {
        let since = self.running_since?;
        if self.has_next() {
            Some(
                since
                    .saturating_add(self.due_millis(self.next))
                    .saturating_sub(refresh_compensation_ms),
            )
        } else {
            Some(since.saturating_add(self.due_millis(self.frames())))
        }
    }

    /// Start or resume playback on the declared clock and start decoded PCM
    /// audio on `mixer` under `voice_id` if the movie carries an audio stream.
    ///
    /// # Errors
    /// Returns [`sound::Error`] if the movie's audio track fails to decode.
    /// Playback state is left unchanged on error.
    pub fn start(
        &mut self,
        voice_id: u32,
        now_ms: u64,
        mixer: &mut Mixer,
    ) -> Result<StartOutcome, sound::Error> {
        if !self.has_next() {
            let first_eos = !self.completed;
            self.completed = true;
            let fallback = now_ms.saturating_sub(self.due_millis(self.next));
            self.running_since.get_or_insert(fallback);
            return Ok(StartOutcome::AtEnd { first_eos });
        }
        if self.running_since.is_some() {
            return Ok(StartOutcome::AlreadyRunning);
        }
        let since = now_ms.saturating_sub(self.due_millis(self.next));
        let samples = self.avi.decode_audio(&self.bytes)?;
        self.running_since = Some(since);
        self.completed = false;
        if let Some(samples) = samples {
            mixer.play(voice_id, samples, since * 1_000_000);
        }
        Ok(StartOutcome::Started)
    }

    /// Stop playback and silence `voice_id` on `mixer` at `now_ms`.
    pub fn stop(&mut self, voice_id: u32, now_ms: u64, mixer: &mut Mixer) {
        self.running_since = None;
        mixer.stop(voice_id, now_ms * 1_000_000);
    }

    /// Rewind to frame 0, resetting the IV50 decoder and restarting `mixer`
    /// audio if currently running. Returns `true` when running with at least
    /// one frame to stage.
    ///
    /// # Errors
    /// Returns [`sound::Error`] if restarting audio fails (leaving state intact).
    pub fn rewind(
        &mut self,
        voice_id: u32,
        now_ms: u64,
        mixer: &mut Mixer,
    ) -> Result<bool, sound::Error> {
        let was_running = self.running_since.is_some();
        let samples = if was_running {
            self.avi.decode_audio(&self.bytes)?
        } else {
            None
        };
        self.decoder = Decoder::new();
        self.next = 0;
        self.completed = false;
        if was_running {
            self.running_since = Some(now_ms);
            mixer.stop(voice_id, now_ms * 1_000_000);
            if let Some(samples) = samples {
                mixer.play(voice_id, samples, now_ms * 1_000_000);
            }
        }
        Ok(was_running && self.frames() > 0)
    }

    /// Decode the next IV50 video frame and blit it into a guest RGB565 surface
    /// at `(pixels, width, height, pitch)`.
    ///
    /// Returns `Ok(true)` when a frame was rendered, or `Ok(false)` at
    /// end-of-stream. Empty chunks repeat the previous decoded picture.
    ///
    /// # Errors
    /// Returns [`StepError::Decode`] on corrupt IV50 data or [`StepError::Fault`]
    /// if writing to guest memory faults.
    pub fn step_rgb565(
        &mut self,
        mem: &mut Arena,
        pixels: u32,
        width: u32,
        height: u32,
        pitch: u32,
    ) -> Result<bool, StepError> {
        if !self.has_next() {
            return Ok(false);
        }
        let index = self.next;
        let (at, len) = self.avi.streams[self.video].chunks[index];
        self.next += 1;
        if len > 0 {
            self.decoder
                .decode(&self.bytes[at..at + len], &mut self.picture)
                .map_err(|error| StepError::Decode { index, error })?;
        }
        write_rgb565(mem, &self.picture, pixels, width, height, pitch).map_err(StepError::Fault)?;
        Ok(true)
    }
}

/// Write a decoded `iv50::Picture` into a guest RGB565 surface buffer.
fn write_rgb565(
    mem: &mut Arena,
    picture: &Picture,
    pixels: u32,
    width: u32,
    height: u32,
    pitch: u32,
) -> Result<(), Fault> {
    let w = (width as usize).min(picture.width);
    let h = (height as usize).min(picture.height);
    let mut row = vec![0u8; w * 2];
    for y in 0..h {
        for x in 0..w {
            let (r, g, b) = picture.rgb(x, y);
            let pixel = (u16::from(r >> 3) << 11) | (u16::from(g >> 2) << 5) | u16::from(b >> 3);
            row[x * 2..x * 2 + 2].copy_from_slice(&pixel.to_le_bytes());
        }
        let dst = u32::try_from(y)
            .ok()
            .and_then(|row_idx| row_idx.checked_mul(pitch))
            .and_then(|off| pixels.checked_add(off))
            .ok_or(Fault {
                addr: pixels,
                len: row.len(),
                write: true,
            })?;
        mem.write(dst, &row)?;
    }
    Ok(())
}

impl Snap for MoviePlayer {
    fn snap(&mut self, v: &mut dyn Visitor) {
        // `bytes` is an immutable `Rc<[u8]>` rehydrated via `key` from
        // `MovieStore` / `Tree` on restore, keeping snapshots free of raw AVI bytes.
        let Self {
            name,
            key,
            bytes,
            avi,
            video,
            decoder,
            picture,
            next,
            running_since,
            completed,
        } = self;
        if v.is_reading() {
            *bytes = Rc::from([]);
        }
        v.string(name);
        v.string(key);
        avi.snap(v);
        v.usize(video);
        decoder.snap(v);
        picture.snap(v);
        v.usize(next);
        crate::snap::snap_option(v, running_since, |r, v| v.u64(r));
        v.bool(completed);
        if v.is_reading() && !v.failed() {
            let (cw, ch) = picture.chroma_size();
            let luma_needed = picture.width.checked_mul(picture.height);
            let chroma_needed = cw.checked_mul(ch);
            let picture_valid = luma_needed.is_some_and(|n| picture.y.len() >= n)
                && chroma_needed.is_some_and(|n| picture.u.len() >= n && picture.v.len() >= n);
            if key.trim().is_empty() {
                v.mark_corrupt("MoviePlayer empty key");
            } else if *video >= avi.streams.len() {
                v.mark_corrupt("MoviePlayer video stream index");
            } else if *next > avi.streams[*video].chunks.len() {
                v.mark_corrupt("MoviePlayer frame index");
            } else if !picture_valid {
                v.mark_corrupt("MoviePlayer picture planes");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{write_rgb565, MoviePlayer, MovieStore, OpenError, StartOutcome, StepError};
    use crate::guest::{Arena, GuestMem};
    use crate::host::headless::Headless;
    use crate::install::Tree;
    use crate::iv50::Picture;
    use crate::snap::{Reader, Snap, Writer};
    use crate::sound::Mixer;
    use std::rc::Rc;

    fn chunk(id: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = id.to_vec();
        out.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        out.extend_from_slice(body);
        if body.len() % 2 == 1 {
            out.push(0);
        }
        out
    }

    fn list(kind: &[u8; 4], parts: &[Vec<u8>]) -> Vec<u8> {
        let mut body = kind.to_vec();
        for p in parts {
            body.extend_from_slice(p);
        }
        chunk(b"LIST", &body)
    }

    fn synthetic_avi(
        codec: Option<&[u8; 4]>,
        frames: usize,
        with_audio: bool,
        junk_bytes: usize,
    ) -> Vec<u8> {
        let mut hdrl_parts = Vec::new();
        let mut movi_parts = Vec::new();
        if let Some(codec) = codec {
            let mut strh_v = vec![0u8; 56];
            strh_v[0..4].copy_from_slice(b"vids");
            strh_v[4..8].copy_from_slice(codec);
            strh_v[20..24].copy_from_slice(&1u32.to_le_bytes());
            strh_v[24..28].copy_from_slice(&15u32.to_le_bytes());
            strh_v[32..36].copy_from_slice(&(frames as u32).to_le_bytes());
            let mut strf_v = vec![0u8; 40];
            strf_v[0..4].copy_from_slice(&40u32.to_le_bytes());
            strf_v[4..8].copy_from_slice(&16u32.to_le_bytes());
            strf_v[8..12].copy_from_slice(&16u32.to_le_bytes());
            strf_v[14..16].copy_from_slice(&24u16.to_le_bytes());
            strf_v[16..20].copy_from_slice(codec);
            hdrl_parts.push(list(
                b"strl",
                &[chunk(b"strh", &strh_v), chunk(b"strf", &strf_v)],
            ));
        }
        if with_audio {
            let mut strh_a = vec![0u8; 56];
            strh_a[0..4].copy_from_slice(b"auds");
            strh_a[20..24].copy_from_slice(&4u32.to_le_bytes());
            strh_a[24..28].copy_from_slice(&(22_050u32 * 4).to_le_bytes());
            strh_a[44..48].copy_from_slice(&4u32.to_le_bytes());
            let mut strf_a = vec![0u8; 16];
            strf_a[0..2].copy_from_slice(&1u16.to_le_bytes());
            strf_a[2..4].copy_from_slice(&2u16.to_le_bytes());
            strf_a[4..8].copy_from_slice(&22_050u32.to_le_bytes());
            strf_a[8..12].copy_from_slice(&(22_050u32 * 4).to_le_bytes());
            strf_a[12..14].copy_from_slice(&4u16.to_le_bytes());
            strf_a[14..16].copy_from_slice(&16u16.to_le_bytes());
            hdrl_parts.push(list(
                b"strl",
                &[chunk(b"strh", &strh_a), chunk(b"strf", &strf_a)],
            ));

            let mut pcm = Vec::with_capacity(400);
            for i in 0..100i16 {
                let s = (i * 100).to_le_bytes();
                pcm.extend_from_slice(&s);
                pcm.extend_from_slice(&s);
            }
            let tag = if codec.is_some() { b"01wb" } else { b"00wb" };
            movi_parts.push(chunk(tag, &pcm));
        }
        if codec.is_some() {
            for _ in 0..frames {
                movi_parts.push(chunk(b"00dc", &[]));
            }
        }

        let hdrl = list(b"hdrl", &hdrl_parts);
        let movi = list(b"movi", &movi_parts);
        let mut riff = b"AVI ".to_vec();
        riff.extend_from_slice(&hdrl);
        if junk_bytes > 0 {
            riff.extend_from_slice(&chunk(b"JUNK", &vec![0x5au8; junk_bytes]));
        }
        riff.extend_from_slice(&movi);
        chunk(b"RIFF", &riff)
    }

    #[test]
    fn movie_store_deduplicates_rehydrates_and_evicts_by_refcount() {
        let mut tree = Tree::new();
        let data_a = b"RIFF-payload-A".to_vec();
        let data_b = b"RIFF-payload-B".to_vec();
        tree.declare("C:\\Game\\Intro.avi", data_a.clone());
        tree.declare("C:\\Game\\Outro.avi", data_b.clone());

        let mut store = MovieStore::default();
        assert!(store.is_empty());
        assert!(store.load("C:\\Game\\Missing.avi", &mut tree).is_none());

        // Casing, separators, and `.`/`..` collapse to one Rc allocation.
        let (k1, b1) = store
            .load("c:/game/./sub/../intro.avi", &mut tree)
            .expect("first");
        let (k2, b2) = store
            .load("C:\\GAME\\INTRO.AVI", &mut tree)
            .expect("second");
        assert_eq!(k1, "C:\\GAME\\INTRO.AVI");
        assert_eq!(k1, k2);
        assert_eq!(store.len(), 1);
        assert!(Rc::ptr_eq(&b1, &b2));
        assert_eq!(Rc::strong_count(&b1), 3);

        // Eviction retains entries held externally and drops only unreferenced ones.
        let (kb, bb) = store.load("C:\\Game\\Outro.avi", &mut tree).expect("outro");
        assert_eq!(store.len(), 2);
        drop(b1);
        drop(b2);
        store.evict_unused();
        assert!(!store.contains(&k1));
        assert!(store.contains(&kb));
        assert_eq!(store.len(), 1);

        drop(bb);
        store.evict_unused();
        assert!(store.is_empty());

        // Cold resolve_for_restore loads from Tree and caches; second call hits cache.
        let r1 = store.resolve_for_restore(&k1, &mut tree).expect("cold");
        assert_eq!(&*r1, &data_a[..]);
        let r2 = store.resolve_for_restore(&k1, &mut tree).expect("warm");
        assert!(Rc::ptr_eq(&r1, &r2));
        assert!(store.resolve_for_restore("", &mut tree).is_err());
        assert!(store.resolve_for_restore("   ", &mut tree).is_err());
        assert!(store
            .resolve_for_restore("C:\\GAME\\ABSENT.AVI", &mut tree)
            .is_err());
    }

    #[test]
    fn movie_player_open_validates_and_evicts_on_error() {
        let mut tree = Tree::new();
        tree.declare("C:\\GAME\\BAD.AVI", b"not a riff".to_vec());
        tree.declare("C:\\GAME\\AUDIO.AVI", synthetic_avi(None, 0, true, 0));
        tree.declare(
            "C:\\GAME\\VID1.AVI",
            synthetic_avi(Some(b"CRAM"), 2, false, 0),
        );

        let mut store = MovieStore::default();
        assert_eq!(
            MoviePlayer::open(
                "miss.avi".into(),
                "C:\\GAME\\MISS.AVI",
                &mut store,
                &mut tree
            )
            .err(),
            Some(OpenError::Missing)
        );
        assert!(matches!(
            MoviePlayer::open("bad.avi".into(), "C:\\GAME\\BAD.AVI", &mut store, &mut tree),
            Err(OpenError::Parse(_))
        ));
        assert!(store.is_empty());

        assert_eq!(
            MoviePlayer::open(
                "audio.avi".into(),
                "C:\\GAME\\AUDIO.AVI",
                &mut store,
                &mut tree
            )
            .err(),
            Some(OpenError::NoVideo)
        );
        assert!(store.is_empty());

        assert_eq!(
            MoviePlayer::open(
                "vid1.avi".into(),
                "C:\\GAME\\VID1.AVI",
                &mut store,
                &mut tree
            )
            .err(),
            Some(OpenError::UnsupportedCodec(*b"CRAM"))
        );
        assert!(store.is_empty());
    }

    #[test]
    fn movie_player_start_stop_step_eos_rewind_and_deadlines() {
        let mut tree = Tree::new();
        let avi = synthetic_avi(Some(b"IV50"), 2, true, 0);
        tree.declare("C:\\GAME\\INTRO.AVI", avi);

        let mut store = MovieStore::default();
        let mut player = MoviePlayer::open(
            "intro.avi".into(),
            "C:\\GAME\\INTRO.AVI",
            &mut store,
            &mut tree,
        )
        .expect("open");
        assert_eq!(player.picture_size(), (16, 16));
        assert_eq!(player.frames(), 2);
        assert!(!player.is_running());
        assert_eq!(player.next_deadline_millis(0), None);
        assert!((player.duration_secs() - (2.0 / 15.0)).abs() < 1e-9);
        assert_eq!(player.frame_for_secs(0.0), 0);
        assert_eq!(player.frame_for_secs(0.1), 1);

        let mut mixer = Mixer::default();
        let mut arena = Arena::new(&mut Headless::new(), 0x1_0000).expect("arena");

        assert_eq!(
            player.start(1, 100, &mut mixer).expect("start"),
            StartOutcome::Started
        );
        assert!(player.is_running());
        assert_eq!(player.running_since(), Some(100));
        assert_eq!(
            player.start(1, 120, &mut mixer).expect("already"),
            StartOutcome::AlreadyRunning
        );
        assert_eq!(player.elapsed_millis(150), 50);

        assert_eq!(player.next_deadline_millis(0), Some(100));
        assert!(player
            .step_rgb565(&mut arena, 0x2000, 16, 16, 32)
            .expect("frame 0"));
        assert_eq!(player.next_frame(), 1);
        assert_eq!(player.next_deadline_millis(16), Some(100 + 66 - 16));
        assert!(player
            .step_rgb565(&mut arena, 0x2000, 16, 16, 32)
            .expect("frame 1"));
        assert!(!player.has_next());
        assert_eq!(player.next_deadline_millis(16), Some(100 + 133));
        assert!(!player
            .step_rgb565(&mut arena, 0x2000, 16, 16, 32)
            .expect("eos"));

        // Starting at EOS reports first_eos=true once and preserves existing anchor.
        assert_eq!(
            player.start(1, 300, &mut mixer).expect("at end 1"),
            StartOutcome::AtEnd { first_eos: true }
        );
        assert!(!player.is_running());
        assert_eq!(player.running_since(), Some(100));
        assert_eq!(
            player.start(1, 350, &mut mixer).expect("at end 2"),
            StartOutcome::AtEnd { first_eos: false }
        );
        assert_eq!(player.running_since(), Some(100));

        assert!(player.rewind(1, 400, &mut mixer).expect("rewind"));
        assert_eq!(player.next_frame(), 0);
        assert!(player.is_running());
        assert_eq!(player.running_since(), Some(400));

        player.stop(1, 450, &mut mixer);
        assert!(!player.is_running());
        assert!(!player.rewind(1, 460, &mut mixer).expect("stopped rewind"));
        assert_eq!(player.running_since(), None);
    }

    #[test]
    fn write_rgb565_encodes_pixels_and_faults_on_bounds_or_overflow() {
        let mut arena = Arena::new(&mut Headless::new(), 0x4000).expect("arena");
        let picture = Picture {
            width: 2,
            height: 2,
            y: vec![81, 16, 16, 81],
            u: vec![90],
            v: vec![240],
        };
        write_rgb565(&mut arena, &picture, 0x2000, 2, 2, 8).expect("blit");
        let (r0, g0, b0) = picture.rgb(0, 0);
        let expected0 = (u16::from(r0 >> 3) << 11) | (u16::from(g0 >> 2) << 5) | u16::from(b0 >> 3);
        let (r1, g1, b1) = picture.rgb(1, 0);
        let expected1 = (u16::from(r1 >> 3) << 11) | (u16::from(g1 >> 2) << 5) | u16::from(b1 >> 3);
        assert_eq!(arena.read_u16(0x2000).unwrap(), expected0);
        assert_eq!(arena.read_u16(0x2002).unwrap(), expected1);
        assert_eq!(arena.read_u16(0x2008).unwrap(), expected1);
        assert_eq!(arena.read_u16(0x200a).unwrap(), expected0);

        assert!(write_rgb565(&mut arena, &picture, 0x3ffc, 2, 2, 8).is_err());
        assert!(write_rgb565(&mut arena, &picture, u32::MAX - 2, 2, 2, 8).is_err());
    }

    #[test]
    fn movie_player_snap_is_zero_payload_and_rehydrates_safely() {
        let mut tree = Tree::new();
        let avi = synthetic_avi(Some(b"IV50"), 3, true, 65_536);
        tree.declare("C:\\GAME\\BIG.AVI", avi.clone());

        let mut store = MovieStore::default();
        let mut player =
            MoviePlayer::open("big.avi".into(), "C:\\GAME\\BIG.AVI", &mut store, &mut tree)
                .expect("open");
        let mut mixer = Mixer::default();
        let mut arena = Arena::new(&mut Headless::new(), 0x1_0000).expect("arena");
        player.start(7, 50, &mut mixer).expect("start");
        player
            .step_rgb565(&mut arena, 0x2000, 16, 16, 32)
            .expect("step 0");

        let mut writer = Writer::new();
        player.snap(&mut writer);
        let encoded = writer.out;
        assert!(
            encoded.len() < 4096 && encoded.len() < avi.len(),
            "MoviePlayer snap must omit raw AVI payload (got {} bytes for {} byte AVI)",
            encoded.len(),
            avi.len()
        );

        let mut restored = MoviePlayer::empty_for_snap();
        let mut reader = Reader::new(&encoded);
        restored.snap(&mut reader);
        reader.finish("movie").expect("valid snap");

        let mut cold_store = MovieStore::default();
        restored
            .rehydrate(&mut cold_store, &mut tree)
            .expect("rehydrate");
        assert!(cold_store.contains("C:\\GAME\\BIG.AVI"));
        assert_eq!(restored.name(), "big.avi");
        assert_eq!(restored.next_frame(), 1);
        assert_eq!(restored.running_since(), Some(50));
        assert!(restored
            .step_rgb565(&mut arena, 0x2000, 16, 16, 32)
            .expect("step 1 after restore"));

        // Truncated backing file on restore must fail cleanly and evict from store.
        let mut bad_tree = Tree::new();
        bad_tree.declare("C:\\GAME\\BIG.AVI", b"tiny".to_vec());
        let mut bad_restored = MoviePlayer::empty_for_snap();
        let mut r2 = Reader::new(&encoded);
        bad_restored.snap(&mut r2);
        r2.finish("movie").expect("snap ok");
        let mut bad_store = MovieStore::default();
        assert!(bad_restored
            .rehydrate(&mut bad_store, &mut bad_tree)
            .is_err());
        assert!(bad_store.is_empty());

        // Corrupt snapshots (empty key, OOB video index, OOB frame index, short picture plane) are rejected.
        let assert_corrupt = |mut bogus: MoviePlayer| {
            let mut w = Writer::new();
            bogus.snap(&mut w);
            let mut r = Reader::new(&w.out);
            let mut out = MoviePlayer::empty_for_snap();
            out.snap(&mut r);
            assert!(r.finish("movie").is_err());
        };
        assert_corrupt(MoviePlayer::empty_for_snap());

        let mut oob_video = MoviePlayer::empty_for_snap();
        oob_video.key = "C:\\GAME\\BIG.AVI".to_owned();
        oob_video.video = 99;
        assert_corrupt(oob_video);

        let mut oob_next = restored;
        oob_next.next = 99;
        assert_corrupt(oob_next);

        let mut bad_pic =
            MoviePlayer::open("big.avi".into(), "C:\\GAME\\BIG.AVI", &mut store, &mut tree)
                .expect("reopen");
        bad_pic.picture = Picture {
            width: 16,
            height: 16,
            y: vec![0; 4],
            u: Vec::new(),
            v: Vec::new(),
        };
        assert_corrupt(bad_pic);

        // Corrupt IV50 chunk surfaces StepError::Decode cleanly.
        let mut corrupt_avi = synthetic_avi(Some(b"IV50"), 1, false, 0);
        let dc_pos = corrupt_avi.windows(4).rposition(|w| w == b"00dc").unwrap();
        let corrupt_chunk = chunk(b"00dc", &[0xff, 0xff, 0xff, 0xff]);
        corrupt_avi.truncate(dc_pos);
        corrupt_avi.extend_from_slice(&corrupt_chunk);
        let riff_len = (corrupt_avi.len() - 8) as u32;
        corrupt_avi[4..8].copy_from_slice(&riff_len.to_le_bytes());
        let movi_pos = corrupt_avi.windows(4).position(|w| w == b"movi").unwrap() - 8;
        let movi_len = (corrupt_avi.len() - movi_pos - 8) as u32;
        corrupt_avi[movi_pos + 4..movi_pos + 8].copy_from_slice(&movi_len.to_le_bytes());
        tree.declare("C:\\BAD.AVI", corrupt_avi);
        let mut corrupt_player =
            MoviePlayer::open("bad.avi".into(), "C:\\BAD.AVI", &mut store, &mut tree)
                .expect("parsed");
        assert!(matches!(
            corrupt_player.step_rgb565(&mut arena, 0x2000, 16, 16, 32),
            Err(StepError::Decode { index: 0, .. })
        ));
    }
}
