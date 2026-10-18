//! The Linux audio device, for `--audio` (ADR-0001, #108).
//!
//! ALSA's PCM interface, reached through `dlopen("libasound.so.2")`, playing
//! on `default`: on a desktop that is PipeWire's or PulseAudio's ALSA plugin,
//! and the sound card itself where there is neither. The machine mixes whether
//! or not this exists. It is fed as the Mac's audio queue is ([`super::feed`]):
//! a thread of its own hands the device a buffer at a time, in writes that
//! wait for the device to have room, so the device paces it, and each buffer
//! is filled from the samples waiting.
//!
//! Compiled for x86-64 and aarch64 Linux, and not yet run on either (#108).

use super::feed::{add_waiting, fill_frames, Waiting, BUFFERS, BUFFER_FRAMES, FRAME_BYTES};
use super::{sys, HostError};
use crate::sound::RATE;
use std::ffi::{c_char, c_int, c_long, c_uint, c_ulong, c_void};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;

/// `SND_PCM_STREAM_PLAYBACK`, `SND_PCM_FORMAT_S16_LE` and
/// `SND_PCM_ACCESS_RW_INTERLEAVED`: the mix's own samples, two channels
/// interleaved, written rather than mapped.
const PLAYBACK: c_int = 0;
const S16_LE: c_int = 2;
const RW_INTERLEAVED: c_int = 3;

/// `-EPIPE`, which is how a write says the device ran dry before it came.
const RAN_DRY: c_long = -32;

/// A PCM, which only ALSA looks inside.
type Pcm = *mut c_void;

/// libasound's entry points, by name, with the signatures `asoundlib.h`
/// gives them.
struct Alsa {
    /// Kept open for as long as any of the functions may be called.
    _lib: sys::Library,
    open: unsafe extern "C" fn(*mut Pcm, *const c_char, c_int, c_int) -> c_int,
    set_params: unsafe extern "C" fn(Pcm, c_int, c_int, c_uint, c_uint, c_int, c_uint) -> c_int,
    writei: unsafe extern "C" fn(Pcm, *const c_void, c_ulong) -> c_long,
    recover: unsafe extern "C" fn(Pcm, c_int, c_int) -> c_int,
    drop: unsafe extern "C" fn(Pcm) -> c_int,
    close: unsafe extern "C" fn(Pcm) -> c_int,
}

impl Alsa {
    fn open() -> Result<Self, HostError> {
        let lib = sys::Library::open("libasound.so.2")?;
        // Safety: each type is the signature ALSA's headers give the function
        // of that name.
        unsafe {
            Ok(Self {
                open: lib.function("snd_pcm_open")?,
                set_params: lib.function("snd_pcm_set_params")?,
                writei: lib.function("snd_pcm_writei")?,
                recover: lib.function("snd_pcm_recover")?,
                drop: lib.function("snd_pcm_drop")?,
                close: lib.function("snd_pcm_close")?,
                _lib: lib,
            })
        }
    }
}

/// An open PCM and the library it came from. Closed when dropped.
struct Device {
    alsa: Alsa,
    pcm: Pcm,
}

// Safety: a device is only ever used from one thread at a time. It is opened
// on the run's, then handed whole to the thread that feeds it, which alone
// touches it from then on and closes it.
unsafe impl Send for Device {}

impl Drop for Device {
    fn drop(&mut self) {
        // Safety: a PCM ALSA opened, stopped without playing what is queued,
        // then closed once.
        unsafe {
            (self.alsa.drop)(self.pcm);
            (self.alsa.close)(self.pcm);
        }
    }
}

/// What the feeding thread and the run share.
struct Shared {
    waiting: Mutex<Waiting>,
    stop: AtomicBool,
}

impl Shared {
    fn waiting(&self) -> MutexGuard<'_, Waiting> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// ALSA's default output, playing the mix.
pub struct LinuxAudio {
    shared: Arc<Shared>,
    feeder: Option<JoinHandle<()>>,
}

/// An ALSA status as a result: negative is an errno, refused.
fn status(what: &'static str, status: c_int) -> Result<(), HostError> {
    if status < 0 {
        Err(HostError::Status { what, status })
    } else {
        Ok(())
    }
}

impl LinuxAudio {
    /// Open the default output and start feeding it silence.
    ///
    /// # Errors
    /// When libasound is not there, or the output will not open at the
    /// mix's rate and format.
    pub fn open() -> Result<Self, HostError> {
        let alsa = Alsa::open()?;
        let mut pcm: Pcm = std::ptr::null_mut();
        // Safety: a name ALSA knows on every system, and a blocking open.
        status("snd_pcm_open", unsafe {
            (alsa.open)(&raw mut pcm, c"default".as_ptr(), PLAYBACK, 0)
        })?;
        // From here the PCM is closed if anything below fails.
        let device = Device { alsa, pcm };
        // As much as the Mac's queue holds: its buffers, about 70 ms. ALSA
        // may resample to what the card runs at.
        let latency = u64::from(BUFFER_FRAMES) * BUFFERS as u64 * 1_000_000 / u64::from(RATE);
        // Safety: a PCM just opened, set up once.
        status("snd_pcm_set_params", unsafe {
            (device.alsa.set_params)(
                pcm,
                S16_LE,
                RW_INTERLEAVED,
                2,
                RATE,
                1,
                c_uint::try_from(latency).unwrap_or(c_uint::MAX),
            )
        })?;
        let shared = Arc::new(Shared {
            waiting: Mutex::new(Waiting::default()),
            stop: AtomicBool::new(false),
        });
        let feeding = Arc::clone(&shared);
        let feeder = std::thread::Builder::new()
            .name("lego-legacy audio".to_owned())
            .spawn(move || feed(&device, &feeding))
            .map_err(|_| HostError::Unsupported("no thread to feed the audio device from"))?;
        Ok(Self {
            shared,
            feeder: Some(feeder),
        })
    }

    /// Take one block of the mix, to be heard as soon as the device gets to
    /// it.
    pub fn play(&self, samples: &[i16]) {
        add_waiting(&mut self.shared.waiting(), samples);
    }

    /// How many times the device has run dry mid-sound. Zero is a mix that
    /// kept up (#75).
    #[must_use]
    pub fn underruns(&self) -> u64 {
        self.shared.waiting().underruns
    }
}

impl Drop for LinuxAudio {
    fn drop(&mut self) {
        // The thread stops at its next buffer, a few tens of milliseconds
        // away, and closes the device as it goes.
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(feeder) = self.feeder.take() {
            let _ = feeder.join();
        }
    }
}

/// Hand the device a buffer at a time until told to stop.
///
/// A device ALSA cannot bring back has nowhere to be reported from this
/// thread; the thread then stops, and the run carries on silent, as on the
/// Mac.
fn feed(device: &Device, shared: &Shared) {
    let frame = FRAME_BYTES as usize;
    let mut bytes = vec![0u8; BUFFER_FRAMES as usize * frame];
    while !shared.stop.load(Ordering::Relaxed) {
        fill_frames(&mut shared.waiting(), &mut bytes);
        let mut left = &bytes[..];
        while !left.is_empty() {
            // Safety: a live PCM, and whole frames behind the pointer.
            let wrote = unsafe {
                (device.alsa.writei)(
                    device.pcm,
                    left.as_ptr().cast(),
                    (left.len() / frame) as c_ulong,
                )
            };
            if let Ok(frames) = usize::try_from(wrote) {
                left = left.get(frames * frame..).unwrap_or_default();
                continue;
            }
            // The device ran dry before this thread came back to it, which a
            // person hears as a gap just as they hear the mix running out:
            // counted with those (#75).
            if wrote == RAN_DRY {
                shared.waiting().underruns += 1;
            }
            let error = c_int::try_from(wrote).unwrap_or(c_int::MIN);
            // Safety: a live PCM, and the error it gave.
            if unsafe { (device.alsa.recover)(device.pcm, error, 1) } < 0 {
                return;
            }
        }
    }
}
