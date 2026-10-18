//! Windows' audio device, for `--audio` (ADR-0001, #107).
//!
//! winmm's `waveOut`, loaded by name at run time: the simplest output a
//! machine clock can pace, and on every Windows there is. The machine mixes
//! whether or not this exists. The blocks the mix hands the host wait in the
//! same feed macOS's audio queue reads from (`feed`), and a thread of ours
//! refills each buffer the device hands back, as the queue's own thread does
//! on macOS. Compiled for Windows and never yet run there.

use super::feed::{add_waiting, fill_frames, Waiting, FRAME_BYTES};
use super::{sys, HostError};
use crate::sound::RATE;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

type Handle = *mut c_void;

/// The device Windows would choose, which is the person's default output.
const WAVE_MAPPER: u32 = u32::MAX;
const WAVE_FORMAT_PCM: u16 = 1;
/// The device signals an event of ours each time it is done with a buffer.
const CALLBACK_EVENT: u32 = 0x0005_0000;
/// A buffer's flag for done with, by the device.
const WHDR_DONE: u32 = 0x0000_0001;

/// Frames in one of the device's buffers, about 23 ms at the mix's rate, as
/// on macOS.
const BUFFER_FRAMES: usize = 512;

/// Buffers the device holds at once: one more than macOS's queue, because a
/// buffer is refilled by a thread woken by Windows rather than by the device
/// itself.
// ponytail: four, untried. If a run's log says the device never ran dry and a
// person still hears gaps, the device is being starved before the feed is,
// and this is the number to raise.
const BUFFERS: usize = 4;

/// How long the feeding thread sleeps between looks when nothing wakes it.
const WAIT_MILLIS: u32 = 50;

/// A `WAVEFORMATEX`.
#[repr(C)]
struct WaveFormatEx {
    format_tag: u16,
    channels: u16,
    samples_per_sec: u32,
    avg_bytes_per_sec: u32,
    block_align: u16,
    bits_per_sample: u16,
    extra_size: u16,
}

/// A `WAVEHDR`.
#[repr(C)]
struct WaveHdr {
    data: *mut u8,
    buffer_length: u32,
    bytes_recorded: u32,
    user: usize,
    flags: u32,
    loops: u32,
    next: *mut WaveHdr,
    reserved: usize,
}

type Open =
    unsafe extern "system" fn(*mut Handle, u32, *const WaveFormatEx, usize, usize, u32) -> u32;
type Header = unsafe extern "system" fn(Handle, *mut WaveHdr, u32) -> u32;
type Device = unsafe extern "system" fn(Handle) -> u32;

#[link(name = "kernel32")]
extern "system" {
    fn CreateEventW(
        attributes: *mut c_void,
        manual_reset: i32,
        initial: i32,
        name: *const u16,
    ) -> Handle;
    fn SetEvent(event: Handle) -> i32;
    fn WaitForSingleObject(handle: Handle, millis: u32) -> u32;
    fn CloseHandle(handle: Handle) -> i32;
}

/// What the feeding thread and the run share: the samples waiting to be
/// heard, and whether to stop.
struct Shared {
    waiting: Mutex<Waiting>,
    stop: AtomicBool,
}

impl Shared {
    fn waiting(&self) -> MutexGuard<'_, Waiting> {
        self.waiting.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// What the feeding thread holds: the device, its event, its buffers, and
/// the call that hands a buffer back.
struct Feeder {
    device: Handle,
    event: Handle,
    headers: *mut WaveHdr,
    write: Header,
    shared: Arc<Shared>,
}

// Safety: the device and event handles may be used from any thread, and the
// headers are only touched by the device and by this thread until the thread
// is joined, after which only by `WinAudio`'s drop.
unsafe impl Send for Feeder {}

impl Feeder {
    /// Refill every buffer the device is done with, until told to stop.
    fn run(self) {
        loop {
            // Safety: our own event, live until this thread is joined.
            unsafe { WaitForSingleObject(self.event, WAIT_MILLIS) };
            if self.shared.stop.load(Ordering::Acquire) {
                return;
            }
            for i in 0..BUFFERS {
                // Safety: `headers` holds `BUFFERS` prepared headers, each
                // over `buffer_length` bytes of its own. The device writes a
                // header's flags from its own thread, so they are read as
                // memory something else may have changed; once done, the
                // header is ours until it is written again.
                unsafe {
                    let header = self.headers.add(i);
                    let flags = std::ptr::addr_of!((*header).flags).read_volatile();
                    if flags & WHDR_DONE == 0 {
                        continue;
                    }
                    let bytes = std::slice::from_raw_parts_mut(
                        (*header).data,
                        (*header).buffer_length as usize,
                    );
                    fill_frames(&mut self.shared.waiting(), bytes);
                    (*header).flags = flags & !WHDR_DONE;
                    (self.write)(self.device, header, header_size());
                }
            }
        }
    }
}

fn header_size() -> u32 {
    std::mem::size_of::<WaveHdr>() as u32
}

/// Windows' default output, playing the mix.
pub struct WinAudio {
    device: Handle,
    event: Handle,
    shared: Arc<Shared>,
    /// The device's buffers' headers and their bytes, at addresses that do
    /// not move for as long as the device is open.
    headers: Box<[WaveHdr]>,
    _buffers: Box<[u8]>,
    feeder: Option<std::thread::JoinHandle<()>>,
    prepare: Header,
    unprepare: Header,
    write: Header,
    reset: Device,
    close: Device,
    _winmm: sys::Library,
}

/// An `MMRESULT` as a result.
fn status(what: &'static str, status: u32) -> Result<(), HostError> {
    if status == 0 {
        Ok(())
    } else {
        Err(HostError::Status {
            what,
            status: i32::try_from(status).unwrap_or(i32::MAX),
        })
    }
}

impl WinAudio {
    /// Open the default output and start it playing silence.
    ///
    /// # Errors
    /// When winmm is not there or the device will not open.
    pub fn open() -> Result<Self, HostError> {
        let winmm = sys::Library::open("winmm.dll")?;
        // Safety: each type is the signature mmeapi.h gives the function.
        let (open, prepare, unprepare, write, reset, close) = unsafe {
            (
                winmm.function::<Open>("waveOutOpen")?,
                winmm.function::<Header>("waveOutPrepareHeader")?,
                winmm.function::<Header>("waveOutUnprepareHeader")?,
                winmm.function::<Header>("waveOutWrite")?,
                winmm.function::<Device>("waveOutReset")?,
                winmm.function::<Device>("waveOutClose")?,
            )
        };
        let format = WaveFormatEx {
            format_tag: WAVE_FORMAT_PCM,
            channels: 2,
            samples_per_sec: RATE,
            avg_bytes_per_sec: RATE * FRAME_BYTES,
            block_align: FRAME_BYTES as u16,
            bits_per_sample: 16,
            extra_size: 0,
        };
        // Safety: an unnamed auto-reset event, not yet signalled.
        let event = unsafe { CreateEventW(std::ptr::null_mut(), 0, 0, std::ptr::null()) };
        if event.is_null() {
            return Err(HostError::Os {
                what: "CreateEventW",
                errno: std::io::Error::last_os_error().raw_os_error().unwrap_or(0),
            });
        }
        let mut device: Handle = std::ptr::null_mut();
        // Safety: the device copies the format; the event lives until the
        // device is closed.
        let opened = unsafe {
            open(
                &mut device,
                WAVE_MAPPER,
                &format,
                event as usize,
                0,
                CALLBACK_EVENT,
            )
        };
        if let Err(e) = status("waveOutOpen", opened) {
            // Safety: no device holds the event.
            unsafe { CloseHandle(event) };
            return Err(e);
        }
        let bytes = BUFFER_FRAMES * FRAME_BYTES as usize;
        let mut buffers = vec![0u8; bytes * BUFFERS].into_boxed_slice();
        let headers: Box<[WaveHdr]> = buffers
            .chunks_exact_mut(bytes)
            .map(|chunk| WaveHdr {
                data: chunk.as_mut_ptr(),
                buffer_length: bytes as u32,
                bytes_recorded: 0,
                user: 0,
                flags: 0,
                loops: 0,
                next: std::ptr::null_mut(),
                reserved: 0,
            })
            .collect();
        // From here the device is reset and closed if anything below fails.
        let mut audio = Self {
            device,
            event,
            shared: Arc::new(Shared {
                waiting: Mutex::new(Waiting::default()),
                stop: AtomicBool::new(false),
            }),
            headers,
            _buffers: buffers,
            feeder: None,
            prepare,
            unprepare,
            write,
            reset,
            close,
            _winmm: winmm,
        };
        // The first buffers are silence, and writing them starts the device;
        // each one it finishes is refilled from what is waiting.
        for header in audio.headers.iter_mut() {
            // Safety: an open device, and a header over bytes of its own that
            // stay where they are until the device is closed.
            unsafe {
                status(
                    "waveOutPrepareHeader",
                    (audio.prepare)(device, header, header_size()),
                )?;
                status("waveOutWrite", (audio.write)(device, header, header_size()))?;
            }
        }
        let feeder = Feeder {
            device,
            event,
            headers: audio.headers.as_mut_ptr(),
            write: audio.write,
            shared: Arc::clone(&audio.shared),
        };
        audio.feeder = Some(
            std::thread::Builder::new()
                .name("lego-legacy audio".to_owned())
                .spawn(move || feeder.run())
                .map_err(|_| HostError::Unsupported("no thread to feed the audio device"))?,
        );
        Ok(audio)
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

impl Drop for WinAudio {
    fn drop(&mut self) {
        // The thread stops first, so that nothing writes a buffer while the
        // device is being reset; then every buffer comes back, and the device
        // closes with none of them queued.
        self.shared.stop.store(true, Ordering::Release);
        // Safety: our own event.
        unsafe { SetEvent(self.event) };
        if let Some(feeder) = self.feeder.take() {
            let _ = feeder.join();
        }
        // Safety: the device is ours and nothing else is using it now; after
        // the reset no buffer is queued, so each can be unprepared, and then
        // the device closed and its event with it.
        unsafe {
            (self.reset)(self.device);
            for header in self.headers.iter_mut() {
                (self.unprepare)(self.device, header, header_size());
            }
            (self.close)(self.device);
            CloseHandle(self.event);
        }
    }
}
